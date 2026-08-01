//! Long-lived Turso connection with one-shot reconnect on connection failure.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use tracing::warn;
use turso::{Builder, Connection, Database};

use super::super::error::LibraryError;

/// Owns a Turso [`Database`] and a replaceable working [`Connection`].
pub(crate) struct ReconnectableDb {
    database: Database,
    conn: Mutex<Connection>,
}

impl ReconnectableDb {
    pub(crate) async fn open(path: &Path) -> Result<Self, LibraryError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let path_string = path.to_string_lossy().into_owned();
        let database = Builder::new_local(&path_string).build().await?;
        let conn = database.connect()?;
        Ok(Self {
            database,
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn connection(&self) -> Connection {
        self.conn
            .lock()
            .expect("library connection mutex poisoned")
            .clone()
    }

    pub(crate) fn reconnect(&self) -> Result<(), LibraryError> {
        let conn = self.database.connect()?;
        *self.conn.lock().expect("library connection mutex poisoned") = conn;
        Ok(())
    }

    /// Run `op` on the current connection; on connection failure, reconnect once and retry.
    pub(crate) async fn run<T, F, Fut>(&self, mut op: F) -> Result<T, LibraryError>
    where
        F: FnMut(Connection) -> Fut,
        Fut: Future<Output = Result<T, LibraryError>>,
    {
        match op(self.connection()).await {
            Ok(value) => Ok(value),
            Err(err) if err.is_connection_failure() => {
                warn!(%err, "library database connection failed; reconnecting");
                self.reconnect()?;
                op(self.connection()).await
            }
            Err(err) => Err(err),
        }
    }
}
