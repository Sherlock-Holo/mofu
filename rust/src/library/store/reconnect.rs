//! Deadpool-managed Turso connections with retries on discardable failures.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use deadpool::Runtime;
use deadpool::managed::{Manager, Metrics, Object, Pool, RecycleError, RecycleResult};
use smol::Timer;
use tracing::warn;
use turso::{Builder, Connection, Database};

use super::super::error::LibraryError;

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long each statement waits on a locked database before returning Busy.
const BUSY_TIMEOUT: Duration = Duration::from_millis(100);
/// Same-connection retries after `BUSY_TIMEOUT` is exhausted (lock contention).
const MAX_BUSY_RETRIES: usize = 20;
/// Extra pause between Busy retries after `BUSY_TIMEOUT` is exhausted.
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);
/// Fresh-pool checkouts after a truly bad connection (misuse / I/O); keep small.
const MAX_DISCARDABLE_RETRIES: usize = 2;

/// Pooled Turso connection with idle-in-pool tracking.
///
/// `last_parked` is the instant the connection was last returned (or created),
/// not checkout duration — so long in-use ops do not look "idle".
struct PooledConn {
    conn: Connection,
    last_parked: Instant,
}

impl PooledConn {
    fn fresh(conn: Connection) -> Self {
        Self {
            conn,
            last_parked: Instant::now(),
        }
    }

    fn is_idle(&self) -> bool {
        self.last_parked.elapsed() > IDLE_TIMEOUT
    }

    fn mark_parked(&mut self) {
        self.last_parked = Instant::now();
    }
}

struct TursoManager {
    database: Database,
}

impl Manager for TursoManager {
    type Type = PooledConn;
    type Error = LibraryError;

    async fn create(&self) -> Result<PooledConn, LibraryError> {
        let conn = self.database.connect()?;
        // Without this, concurrent writers fail immediately with Busy; the outer
        // retry loop alone cannot wait out a lock held by another pool connection.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(PooledConn::fresh(conn))
    }

    async fn recycle(
        &self,
        conn: &mut PooledConn,
        _metrics: &Metrics,
    ) -> RecycleResult<LibraryError> {
        if conn.is_idle() {
            return Err(RecycleError::message("idle timeout"));
        }
        Ok(())
    }
}

/// Owns a Turso [`Database`] behind a deadpool connection pool.
///
/// Each [`Self::run`] checkout uses an independent connection so concurrent
/// library ops do not hit Turso's "concurrent use forbidden" misuse.
pub(crate) struct ReconnectableDb {
    pool: Pool<TursoManager>,
}

impl ReconnectableDb {
    pub(crate) async fn open(path: &Path) -> Result<Self, LibraryError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let path_string = path.to_string_lossy().into_owned();
        let database = Builder::new_local(&path_string).build().await?;
        let pool = Pool::builder(TursoManager { database })
            .wait_timeout(Some(WAIT_TIMEOUT))
            .runtime(Runtime::Smol2)
            .build()?;
        Ok(Self { pool })
    }

    /// Run `op` on a pooled connection.
    ///
    /// Busy/BusySnapshot keep the connection and retry in-place:
    /// first attempt + [`MAX_BUSY_RETRIES`] same-connection retries.
    /// Truly bad connections (misuse / I/O) are discarded and retried with a
    /// fresh checkout: first attempt + [`MAX_DISCARDABLE_RETRIES`] retries.
    ///
    /// Uses [`AsyncFnMut`] (not [`AsyncFn`]): the retry loop calls `op` repeatedly
    /// through `&mut F`. Passes `&mut Connection` so the checked-out pool object
    /// stays exclusively borrowed for the whole op — no connection clone.
    pub(crate) async fn run<T, F>(&self, mut op: F) -> Result<T, LibraryError>
    where
        F: AsyncFnMut(&mut Connection) -> Result<T, LibraryError>,
    {
        // Constants count retries after the first attempt (not total attempts).
        for attempt in 0..MAX_DISCARDABLE_RETRIES {
            match Self::run_once(&self.pool, &mut op).await {
                Ok(value) => return Ok(value),
                Err(err) if err.should_discard_pooled_connection() => {
                    warn!(
                        %err,
                        attempt,
                        "library database connection failed; retrying with a fresh pool connection"
                    );
                }
                Err(err) => return Err(err),
            }
        }
        Self::run_once(&self.pool, &mut op).await
    }

    async fn run_once<T, F>(pool: &Pool<TursoManager>, op: &mut F) -> Result<T, LibraryError>
    where
        F: AsyncFnMut(&mut Connection) -> Result<T, LibraryError>,
    {
        let mut obj = pool.get().await?;
        // Constants count retries after the first attempt (not total attempts).
        for attempt in 0..MAX_BUSY_RETRIES {
            match op(&mut obj.conn).await {
                Ok(value) => {
                    obj.mark_parked();
                    return Ok(value);
                }
                Err(err) if err.is_database_busy() => {
                    warn!(
                        %err,
                        attempt,
                        "library database busy; retrying on the same pooled connection"
                    );
                    Timer::after(BUSY_RETRY_DELAY).await;
                }
                Err(err) if err.should_discard_pooled_connection() => {
                    drop(Object::take(obj));
                    return Err(err);
                }
                Err(err) => {
                    obj.mark_parked();
                    return Err(err);
                }
            }
        }
        match op(&mut obj.conn).await {
            Ok(value) => {
                obj.mark_parked();
                Ok(value)
            }
            Err(err) if err.should_discard_pooled_connection() => {
                drop(Object::take(obj));
                Err(err)
            }
            Err(err) => {
                // Busy or other non-discardable: keep the connection.
                obj.mark_parked();
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn pooled_conn_idle_uses_last_parked_not_checkout_span() {
        futures_executor::block_on(async {
            let (_dir, manager) = test_manager().await;
            let mut conn = PooledConn {
                conn: manager.database.connect().expect("connect"),
                last_parked: Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1),
            };
            assert!(conn.is_idle());
            conn.mark_parked();
            assert!(!conn.is_idle());
        });
    }

    #[test]
    fn recycle_discards_idle_pooled_conn() {
        futures_executor::block_on(async {
            let (_dir, manager) = test_manager().await;
            let mut conn = PooledConn {
                conn: manager.database.connect().expect("connect"),
                last_parked: Instant::now() - IDLE_TIMEOUT - Duration::from_millis(1),
            };
            let err = manager
                .recycle(&mut conn, &Metrics::default())
                .await
                .expect_err("idle should discard");
            assert!(matches!(err, RecycleError::Message(_)));

            conn.mark_parked();
            manager
                .recycle(&mut conn, &Metrics::default())
                .await
                .expect("freshly parked should recycle");
        });
    }

    #[test]
    fn should_discard_covers_misuse_and_io_not_busy() {
        assert!(
            !LibraryError::Database(turso::Error::Busy("locked".into()))
                .should_discard_pooled_connection()
        );
        assert!(
            !LibraryError::Database(turso::Error::BusySnapshot("snapshot".into()))
                .should_discard_pooled_connection()
        );
        assert!(
            LibraryError::Database(turso::Error::Misuse(
                "connection concurrent use forbidden".into()
            ))
            .should_discard_pooled_connection()
        );
        assert!(
            LibraryError::Database(turso::Error::Misuse("inner connection must be set".into()))
                .should_discard_pooled_connection()
        );
        assert!(
            LibraryError::Database(turso::Error::IoError(ErrorKind::BrokenPipe, "read"))
                .should_discard_pooled_connection()
        );
        assert!(
            !LibraryError::Database(turso::Error::Constraint("unique".into()))
                .should_discard_pooled_connection()
        );
        assert!(!LibraryError::PlaylistNotFound.should_discard_pooled_connection());
        assert!(
            LibraryError::Database(turso::Error::Busy("database is locked".into()))
                .is_database_busy()
        );
        assert!(
            !LibraryError::Database(turso::Error::Misuse(
                "connection concurrent use forbidden".into()
            ))
            .is_database_busy()
        );
    }

    #[test]
    fn discardable_failure_retries_with_fresh_connection() {
        futures_executor::block_on(async {
            let dir = TempDir::new().expect("tempdir");
            let db = ReconnectableDb::open(&dir.path().join("retry.db"))
                .await
                .expect("open");
            let mut attempts = 0;
            let value = db
                .run(async |conn| {
                    attempts += 1;
                    if attempts == 1 {
                        let _ = conn;
                        return Err(LibraryError::Database(turso::Error::Misuse(
                            "connection concurrent use forbidden".into(),
                        )));
                    }
                    let mut rows = conn.query("SELECT 1", ()).await?;
                    let row = rows.next().await?.expect("row");
                    let n: i64 = row.get(0)?;
                    Ok(n)
                })
                .await
                .expect("retry succeeds");
            assert_eq!(value, 1);
            assert_eq!(attempts, 2);
        });
    }

    #[test]
    fn discardable_gives_up_after_small_retry_budget() {
        futures_executor::block_on(async {
            let dir = TempDir::new().expect("tempdir");
            let db = ReconnectableDb::open(&dir.path().join("discard-fail.db"))
                .await
                .expect("open");
            let mut attempts = 0;
            let err = db
                .run(async |_conn| {
                    attempts += 1;
                    Err::<i64, _>(LibraryError::Database(turso::Error::Misuse(
                        "connection concurrent use forbidden".into(),
                    )))
                })
                .await
                .expect_err("should exhaust discard retries");
            assert!(err.should_discard_pooled_connection());
            assert_eq!(attempts, MAX_DISCARDABLE_RETRIES + 1);
        });
    }

    #[test]
    fn busy_retries_same_connection_up_to_max_then_succeeds() {
        futures_executor::block_on(async {
            let dir = TempDir::new().expect("tempdir");
            let db = ReconnectableDb::open(&dir.path().join("busy-ok.db"))
                .await
                .expect("open");
            let mut attempts = 0;
            // First attempt + MAX_BUSY_RETRIES Busy failures; succeed on the last retry.
            let succeed_on = MAX_BUSY_RETRIES + 1;
            let value = db
                .run(async |conn| {
                    attempts += 1;
                    if attempts < succeed_on {
                        let _ = conn;
                        return Err(LibraryError::Database(turso::Error::Busy(
                            "database is locked".into(),
                        )));
                    }
                    let mut rows = conn.query("SELECT 1", ()).await?;
                    let row = rows.next().await?.expect("row");
                    let n: i64 = row.get(0)?;
                    Ok(n)
                })
                .await
                .expect("busy retries succeed");
            assert_eq!(value, 1);
            assert_eq!(attempts, succeed_on);
        });
    }

    #[test]
    fn busy_gives_up_after_max_same_connection_retries() {
        futures_executor::block_on(async {
            let dir = TempDir::new().expect("tempdir");
            let db = ReconnectableDb::open(&dir.path().join("busy-fail.db"))
                .await
                .expect("open");
            let mut attempts = 0;
            let err = db
                .run(async |_conn| {
                    attempts += 1;
                    Err::<i64, _>(LibraryError::Database(turso::Error::Busy(
                        "database is locked".into(),
                    )))
                })
                .await
                .expect_err("should exhaust busy retries");
            assert!(err.is_database_busy());
            assert_eq!(attempts, MAX_BUSY_RETRIES + 1);
        });
    }

    async fn test_manager() -> (TempDir, TursoManager) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("manager.db");
        let path_string = path.to_string_lossy().into_owned();
        let database = Builder::new_local(&path_string)
            .build()
            .await
            .expect("build db");
        (dir, TursoManager { database })
    }
}
