//! Confirmed lyrics persistence: sidecar or data-dir files, plus a per-track override index.
//!
//! Lyric bodies are never written to `catalog.db`. A confirmed candidate is written from the
//! in-memory text only; this module does not fetch over the network.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::debug;

use super::dto::PlayerView;
use crate::player::metadata::{Lyrics, LyricsContent, LyricsSource, TrackMetadata};

const INDEX_FILE_NAME: &str = "overrides.json";
const INDEX_VERSION: u32 = 1;

/// User-authorized persist action for a confirmed candidate.
#[flutter_rust_bridge::frb(ignore)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricsConfirmAction {
    /// Default first attempt: same-directory same-stem `.lrc`, or an automatic data-dir fallback.
    Save,
    /// Overwrite an existing sibling sidecar after the API reported [`LyricsConfirmOutcome::NeedsReplace`].
    ReplaceExistingSidecar,
    /// Write a unique data-dir file after a same-stem collision acknowledgement.
    PreferDataDir,
}

/// Where a confirmed lyric body was (or will be) stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricsSaveLocation {
    Sidecar,
    DataDir,
}

/// Successful persist metadata returned to the command layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsSaveInfo {
    pub audio_path: String,
    pub saved_path: PathBuf,
    pub location: LyricsSaveLocation,
    pub replaced_existing: bool,
}

/// Structured confirm result: either a completed save or an authorized follow-up action.
#[flutter_rust_bridge::frb(ignore)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LyricsConfirmOutcome {
    Saved(LyricsSaveInfo),
    NeedsReplace {
        audio_path: String,
        sidecar_path: PathBuf,
    },
    NeedsStemCollisionAck {
        audio_path: String,
        sidecar_path: PathBuf,
        conflicting_audio: Vec<PathBuf>,
    },
}

/// In-memory candidate body and provenance handed to the store. The body is not re-fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsConfirmRequest {
    pub audio_path: String,
    pub text: String,
    pub provider: String,
    pub source_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub action: LyricsConfirmAction,
}

/// Failures while planning, writing, or loading confirmed lyrics.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LyricsStoreError {
    #[error("lyrics data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("audio path is invalid for lyrics storage")]
    InvalidAudioPath,
    #[error("lyrics override index is invalid: {0}")]
    InvalidIndex(String),
    #[error("{operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
}

impl LyricsStoreError {
    fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

/// Persistent lyrics store under `{data_dir}/lyrics`.
///
/// The override index mutex is only taken by confirm/hydrate planning and is
/// never held by the player-watch overlay path. Official lyrics used for
/// overlay live in a separate in-memory cache so progress ticks and playback
/// controls do not wait on disk IO.
#[derive(Clone, Debug)]
pub(crate) struct LyricsStore {
    root: PathBuf,
    inner: Arc<Mutex<Inner>>,
    official: Arc<Mutex<OfficialState>>,
    #[cfg(test)]
    confirm_hold: Arc<ConfirmHold>,
}

/// Test-only gate so application tests can cancel or start a new search
/// after confirm has been requested but before the persist runs.
#[cfg(test)]
#[derive(Debug, Default)]
struct ConfirmHold {
    armed: AtomicBool,
    entered: AtomicBool,
    released: Mutex<bool>,
    cv: Condvar,
}

#[derive(Debug)]
struct Inner {
    index: OverrideIndex,
    revision: u64,
    faults: IoFaults,
}

/// In-memory official lyrics and the last overlaid metadata view.
#[derive(Debug, Default)]
struct OfficialState {
    /// Parsed official lyrics by audio path. `None` means hydrated with no usable override.
    loaded: HashMap<String, Option<Arc<Lyrics>>>,
    overlay: Option<OverlayCache>,
    /// Highest index revision already published into `loaded`. Used to reject
    /// a hydrate that observed a stale revision after a later confirm.
    applied_revision: u64,
}

#[derive(Debug, Clone)]
struct OverlayCache {
    audio_path: String,
    source_ptr: Option<usize>,
    metadata: Arc<TrackMetadata>,
}

#[derive(Debug, Default)]
struct IoFaults {
    fail_lyrics_writes: u32,
    fail_index_writes: u32,
    unwritable_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OverrideIndex {
    version: u32,
    next_id: u64,
    #[serde(default)]
    overrides: HashMap<String, OverrideRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OverrideRecord {
    location: StoredLocation,
    provider: String,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredLocation {
    Sidecar { path: PathBuf },
    DataDir { file: String },
}

#[derive(Debug, Clone)]
enum SavePlan {
    Sidecar { path: PathBuf, replacing: bool },
    DataDir { file: String },
}

impl Default for OverrideIndex {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            next_id: 1,
            overrides: HashMap::new(),
        }
    }
}

impl LyricsStore {
    /// Open (or create) the store under `root`. A missing index is empty; a corrupt index is an error.
    pub(crate) fn open(root: PathBuf) -> Result<Self, LyricsStoreError> {
        fs::create_dir_all(&root)
            .map_err(|err| LyricsStoreError::io("create lyrics data dir", err))?;
        let index = load_index(&root.join(INDEX_FILE_NAME))?;
        let store = Self {
            root,
            inner: Arc::new(Mutex::new(Inner {
                index,
                revision: 0,
                faults: IoFaults::default(),
            })),
            official: Arc::new(Mutex::new(OfficialState::default())),
            #[cfg(test)]
            confirm_hold: Arc::new(ConfirmHold::default()),
        };
        debug!(root = %store.root().display(), "opened lyrics store");
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn open_ephemeral() -> Result<Self, LyricsStoreError> {
        Self::open(
            unique_temp_dir().map_err(|err| LyricsStoreError::io("create temp lyrics dir", err))?,
        )
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Confirm and persist one in-memory candidate according to `request.action`.
    #[cfg(test)]
    pub(crate) fn confirm(
        &self,
        request: LyricsConfirmRequest,
    ) -> Result<LyricsConfirmOutcome, LyricsStoreError> {
        self.confirm_with_abort(request, None)?
            .ok_or_else(|| unreachable!("confirm without an abort flag always returns an outcome"))
    }

    /// Persist a confirmed body unless `abort` is set before the write starts.
    ///
    /// `Ok(None)` means the caller abandoned the confirm (cancel or a new search)
    /// before `execute_plan`. Once the write has started, it is allowed to finish.
    pub(crate) fn confirm_with_abort(
        &self,
        request: LyricsConfirmRequest,
        abort: Option<&AtomicBool>,
    ) -> Result<Option<LyricsConfirmOutcome>, LyricsStoreError> {
        self.maybe_pause_confirm();
        if abort_requested(abort) {
            return Ok(None);
        }
        let audio_path = request.audio_path.as_str();
        validate_audio_path(Path::new(audio_path))?;
        let mut inner = self.inner.lock().expect("lyrics store mutex");
        if abort_requested(abort) {
            return Ok(None);
        }
        let outcome = match plan_save(audio_path, request.action, &inner.index, &inner.faults)? {
            Err(blocked) => {
                if abort_requested(abort) {
                    return Ok(None);
                }
                Ok(Some(blocked))
            }
            Ok(plan) => {
                if abort_requested(abort) {
                    return Ok(None);
                }
                self.execute_plan(&mut inner, &request, plan).map(Some)
            }
        };
        let revision = inner.revision;
        drop(inner);
        if let Ok(Some(LyricsConfirmOutcome::Saved(_))) = &outcome {
            self.remember_official(&request, revision);
        }
        outcome
    }

    #[cfg(test)]
    pub(crate) fn arm_confirm_hold(&self) {
        let hold = &self.confirm_hold;
        *hold.released.lock().expect("confirm hold mutex") = false;
        hold.entered.store(false, Ordering::Release);
        hold.armed.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn wait_confirm_hold_entered(&self) {
        let hold = &self.confirm_hold;
        let mut released = hold.released.lock().expect("confirm hold mutex");
        while !hold.entered.load(Ordering::Acquire) {
            released = hold.cv.wait(released).expect("confirm hold condvar");
        }
    }

    #[cfg(test)]
    pub(crate) fn release_confirm_hold(&self) {
        let hold = &self.confirm_hold;
        *hold.released.lock().expect("confirm hold mutex") = true;
        hold.cv.notify_all();
    }

    fn maybe_pause_confirm(&self) {
        #[cfg(test)]
        {
            let hold = &self.confirm_hold;
            if !hold.armed.swap(false, Ordering::AcqRel) {
                return;
            }
            let mut released = hold.released.lock().expect("confirm hold mutex");
            hold.entered.store(true, Ordering::Release);
            hold.cv.notify_all();
            while !*released {
                released = hold.cv.wait(released).expect("confirm hold condvar");
            }
        }
    }

    fn execute_plan(
        &self,
        inner: &mut Inner,
        request: &LyricsConfirmRequest,
        plan: SavePlan,
    ) -> Result<LyricsConfirmOutcome, LyricsStoreError> {
        let previous_index = inner.index.clone();
        let bytes = request.text.as_bytes();
        let mut sidecar_backup = None;
        let (final_path, location, replacing) = match plan {
            SavePlan::Sidecar { path, replacing } => {
                if !replacing && path.exists() {
                    return Ok(LyricsConfirmOutcome::NeedsReplace {
                        audio_path: request.audio_path.clone(),
                        sidecar_path: path,
                    });
                }
                match self.write_sidecar(inner, &path, bytes, replacing) {
                    Ok(backup) => {
                        sidecar_backup = backup;
                        (path, LyricsSaveLocation::Sidecar, replacing)
                    }
                    Err(err) if is_unwritable_error(&err) => {
                        let file = take_data_file(&mut inner.index);
                        let fallback = self.data_dir_path(&file)?;
                        self.write_lyrics_file(inner, &fallback, bytes)
                            .map_err(|write_err| {
                                LyricsStoreError::io("write lyrics file", write_err)
                            })?;
                        (fallback, LyricsSaveLocation::DataDir, false)
                    }
                    Err(err) => return Err(LyricsStoreError::io("write lyrics file", err)),
                }
            }
            SavePlan::DataDir { file } => {
                bump_next_id(&mut inner.index, &file);
                let path = self.data_dir_path(&file)?;
                self.write_lyrics_file(inner, &path, bytes)
                    .map_err(|err| LyricsStoreError::io("write lyrics file", err))?;
                (path, LyricsSaveLocation::DataDir, false)
            }
        };

        let record = OverrideRecord {
            location: match location {
                LyricsSaveLocation::Sidecar => StoredLocation::Sidecar {
                    path: final_path.clone(),
                },
                LyricsSaveLocation::DataDir => StoredLocation::DataDir {
                    file: data_file_name_from_path(&final_path)?,
                },
            },
            provider: request.provider.clone(),
            source_id: request.source_id.clone(),
            title: request.title.clone(),
            artist: request.artist.clone(),
        };
        inner
            .index
            .overrides
            .insert(request.audio_path.clone(), record);

        if let Err(err) = self.write_index(inner) {
            inner.index = previous_index;
            rollback_after_index_failure(&final_path, location, sidecar_backup)?;
            return Err(err);
        }

        if let Some(backup) = sidecar_backup {
            let _ = fs::remove_file(backup);
        }
        inner.revision = inner.revision.saturating_add(1);
        Ok(LyricsConfirmOutcome::Saved(LyricsSaveInfo {
            audio_path: request.audio_path.clone(),
            saved_path: final_path,
            location,
            replaced_existing: replacing,
        }))
    }

    fn write_sidecar(
        &self,
        inner: &mut Inner,
        path: &Path,
        bytes: &[u8],
        replacing: bool,
    ) -> io::Result<Option<PathBuf>> {
        if dir_is_marked_unwritable(&inner.faults, path.parent()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected unwritable lyrics directory",
            ));
        }
        let backup = if replacing && path.exists() {
            let backup = backup_path(path);
            fs::copy(path, &backup)?;
            Some(backup)
        } else {
            None
        };
        match self.write_lyrics_file(inner, path, bytes) {
            Ok(()) => Ok(backup),
            Err(err) => {
                if let Some(backup) = backup {
                    let _ = restore_from_backup(&backup, path);
                } else if path.exists() && !replacing {
                    let _ = fs::remove_file(path);
                }
                Err(err)
            }
        }
    }

    fn write_lyrics_file(&self, inner: &mut Inner, path: &Path, bytes: &[u8]) -> io::Result<()> {
        atomic_write(path, bytes, inner.take_lyrics_fail())
    }

    fn write_index(&self, inner: &mut Inner) -> Result<(), LyricsStoreError> {
        let path = self.root.join(INDEX_FILE_NAME);
        let payload = serde_json::to_vec_pretty(&inner.index)
            .map_err(|err| LyricsStoreError::InvalidIndex(err.to_string()))?;
        atomic_write(&path, &payload, inner.take_index_fail())
            .map_err(|err| LyricsStoreError::io("write lyrics override index", err))
    }

    fn data_dir_path(&self, file: &str) -> Result<PathBuf, LyricsStoreError> {
        resolve_data_dir_file(&self.root, file)
    }

    /// Official confirmed lyrics for `audio_path`, if the override target still exists.
    ///
    /// This reads the file and is not safe to call from the player-watch loop.
    pub(crate) fn load_official(&self, audio_path: &str) -> Option<Lyrics> {
        let inner = self.inner.lock().expect("lyrics store mutex");
        self.load_official_locked(&inner, audio_path)
    }

    fn load_official_locked(&self, inner: &Inner, audio_path: &str) -> Option<Lyrics> {
        let record = inner.index.overrides.get(audio_path)?;
        official_from_record(&self.root, record)
    }

    /// Read official lyrics for `audio_path` into the in-memory overlay cache.
    ///
    /// Safe to call from a background task. Returns whether the cache changed.
    pub(crate) fn hydrate_official(&self, audio_path: &str) -> bool {
        let revision = {
            let inner = self.inner.lock().expect("lyrics store mutex");
            inner.revision
        };
        let lyrics = self.load_official(audio_path).map(Arc::new);
        let revision_after = {
            let inner = self.inner.lock().expect("lyrics store mutex");
            inner.revision
        };
        if revision_after != revision {
            return self.hydrate_official(audio_path);
        }
        self.commit_hydrated(audio_path, lyrics, revision)
    }

    /// Install a hydrate result only when no newer confirm has already been
    /// published into the official cache. Does not lock `inner` while holding
    /// the official mutex.
    fn commit_hydrated(
        &self,
        audio_path: &str,
        lyrics: Option<Arc<Lyrics>>,
        observed_revision: u64,
    ) -> bool {
        let mut official = self.official.lock().expect("lyrics official mutex");
        if official.applied_revision > observed_revision {
            return false;
        }
        official.applied_revision = official.applied_revision.max(observed_revision);
        let changed = official.loaded.get(audio_path) != Some(&lyrics);
        official.loaded.insert(audio_path.to_owned(), lyrics);
        if official
            .overlay
            .as_ref()
            .is_some_and(|cache| cache.audio_path == audio_path)
        {
            official.overlay = None;
        }
        changed
    }

    /// Test hook: replay a hydrate commit that observed a stale index revision.
    #[cfg(test)]
    pub(crate) fn apply_stale_hydrate_for_test(
        &self,
        audio_path: &str,
        lyrics: Option<Arc<Lyrics>>,
        observed_revision: u64,
    ) -> bool {
        self.commit_hydrated(audio_path, lyrics, observed_revision)
    }

    fn remember_official(&self, request: &LyricsConfirmRequest, revision: u64) {
        let record = {
            let inner = self.inner.lock().expect("lyrics store mutex");
            if inner.revision != revision {
                return;
            }
            inner.index.overrides.get(&request.audio_path).cloned()
        };
        let Some(record) = record else {
            return;
        };
        let lyrics = Arc::new(Lyrics {
            source: LyricsSource::ExternalFile,
            language: None,
            description: Some(provenance(&record)),
            content: LyricsContent::from_text(request.text.clone()),
        });
        let mut official = self.official.lock().expect("lyrics official mutex");
        official.applied_revision = official.applied_revision.max(revision);
        official
            .loaded
            .insert(request.audio_path.clone(), Some(lyrics));
        official.overlay = None;
    }

    /// Overlay confirmed lyrics onto a player view built from decoder metadata.
    ///
    /// This path never reads the disk and never waits on the confirm/index mutex.
    /// Paths without a hydrated override are left unchanged (embedded-then-sidecar
    /// behavior stays). Returns `Some(path)` when the official cache still needs a
    /// background [`Self::hydrate_official`] call.
    pub(crate) fn apply_official_to_view(&self, view: &mut PlayerView) -> Option<String> {
        let path = view.current_path.clone()?;
        let source_ptr = view.metadata.as_ref().map(arc_ptr);
        let mut official = self.official.lock().expect("lyrics official mutex");
        if let Some(cache) = official.overlay.as_ref()
            && cache.audio_path == path
            && cache.source_ptr == source_ptr
        {
            view.metadata = Some(cache.metadata.clone());
            view.active_lyrics_line = cache
                .metadata
                .active_lyrics_line_index(view.position_ms)
                .map(|index| index as u64);
            return None;
        }
        match official.loaded.get(&path).cloned() {
            None => Some(path),
            Some(None) => {
                official.overlay = None;
                None
            }
            Some(Some(lyrics)) => {
                let mut metadata = view
                    .metadata
                    .as_ref()
                    .map(|meta| meta.as_ref().clone())
                    .unwrap_or_default();
                metadata.lyrics.insert(0, lyrics.as_ref().clone());
                let metadata = Arc::new(metadata);
                view.active_lyrics_line = metadata
                    .active_lyrics_line_index(view.position_ms)
                    .map(|index| index as u64);
                official.overlay = Some(OverlayCache {
                    audio_path: path,
                    source_ptr,
                    metadata: metadata.clone(),
                });
                view.metadata = Some(metadata);
                None
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_lyrics_file_write(&self) {
        self.inner
            .lock()
            .expect("lyrics store mutex")
            .faults
            .fail_lyrics_writes += 1;
    }

    #[cfg(test)]
    pub(crate) fn fail_next_index_write(&self) {
        self.inner
            .lock()
            .expect("lyrics store mutex")
            .faults
            .fail_index_writes += 1;
    }

    #[cfg(test)]
    pub(crate) fn mark_unwritable(&self, dir: &Path) {
        self.inner
            .lock()
            .expect("lyrics store mutex")
            .faults
            .unwritable_dirs
            .push(dir.to_path_buf());
    }

    #[cfg(test)]
    pub(crate) fn has_override(&self, audio_path: &str) -> bool {
        self.inner
            .lock()
            .expect("lyrics store mutex")
            .index
            .overrides
            .contains_key(audio_path)
    }

    #[cfg(test)]
    pub(crate) fn saved_path_for(&self, audio_path: &str) -> Option<PathBuf> {
        let inner = self.inner.lock().expect("lyrics store mutex");
        let record = inner.index.overrides.get(audio_path)?;
        resolved_record_path(&self.root, record).ok()
    }
}

impl Inner {
    fn take_lyrics_fail(&mut self) -> bool {
        if self.faults.fail_lyrics_writes > 0 {
            self.faults.fail_lyrics_writes -= 1;
            true
        } else {
            false
        }
    }

    fn take_index_fail(&mut self) -> bool {
        if self.faults.fail_index_writes > 0 {
            self.faults.fail_index_writes -= 1;
            true
        } else {
            false
        }
    }
}

fn load_index(path: &Path) -> Result<OverrideIndex, LyricsStoreError> {
    match fs::read(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(OverrideIndex::default()),
        Err(err) => Err(LyricsStoreError::io("read lyrics override index", err)),
        Ok(bytes) => {
            if bytes.is_empty() {
                return Ok(OverrideIndex::default());
            }
            let mut index: OverrideIndex = serde_json::from_slice(&bytes)
                .map_err(|err| LyricsStoreError::InvalidIndex(err.to_string()))?;
            if index.version != INDEX_VERSION {
                return Err(LyricsStoreError::InvalidIndex(format!(
                    "unsupported version {}",
                    index.version
                )));
            }
            for record in index.overrides.values() {
                if let StoredLocation::DataDir { file } = &record.location {
                    validate_data_file_name(file)?;
                }
            }
            normalize_next_id(&mut index);
            Ok(index)
        }
    }
}

fn plan_save(
    audio_path: &str,
    action: LyricsConfirmAction,
    index: &OverrideIndex,
    faults: &IoFaults,
) -> Result<Result<SavePlan, LyricsConfirmOutcome>, LyricsStoreError> {
    let audio = Path::new(audio_path);
    let sidecar = sidecar_path(audio)?;
    let parent = audio_parent(audio)?;
    let collisions = stem_collisions(audio)?;
    let existing = index.overrides.get(audio_path);
    let sidecar_exists = sidecar.exists();
    let dir_unwritable = dir_is_marked_unwritable(faults, Some(parent));

    if action == LyricsConfirmAction::PreferDataDir {
        return Ok(Ok(SavePlan::DataDir {
            file: next_data_file(index),
        }));
    }

    if let Some(OverrideRecord {
        location: StoredLocation::DataDir { file },
        ..
    }) = existing
    {
        validate_data_file_name(file)?;
        return Ok(Ok(SavePlan::DataDir {
            file: next_data_file(index),
        }));
    }

    if !collisions.is_empty() {
        return Ok(Err(LyricsConfirmOutcome::NeedsStemCollisionAck {
            audio_path: audio_path.to_owned(),
            sidecar_path: sidecar,
            conflicting_audio: collisions,
        }));
    }

    if dir_unwritable {
        return Ok(Ok(SavePlan::DataDir {
            file: next_data_file(index),
        }));
    }

    if sidecar_exists && action != LyricsConfirmAction::ReplaceExistingSidecar {
        return Ok(Err(LyricsConfirmOutcome::NeedsReplace {
            audio_path: audio_path.to_owned(),
            sidecar_path: sidecar,
        }));
    }

    Ok(Ok(SavePlan::Sidecar {
        path: sidecar,
        replacing: sidecar_exists,
    }))
}

// Never overwrite a data file referenced by the committed index. A failed
// index transaction may leave an orphan, but must not change official lyrics.
fn next_data_file(index: &OverrideIndex) -> String {
    format!("{}.lrc", index.next_id)
}

fn take_data_file(index: &mut OverrideIndex) -> String {
    let file = next_data_file(index);
    bump_next_id(index, &file);
    file
}

fn bump_next_id(index: &mut OverrideIndex, file: &str) {
    if let Some(id) = parse_data_file_id(file) {
        index.next_id = index.next_id.max(id.saturating_add(1));
    }
}

fn normalize_next_id(index: &mut OverrideIndex) {
    let max_id = index
        .overrides
        .values()
        .filter_map(|record| match &record.location {
            StoredLocation::DataDir { file } => parse_data_file_id(file),
            StoredLocation::Sidecar { .. } => None,
        })
        .max()
        .unwrap_or(0);
    index.next_id = index.next_id.max(max_id.saturating_add(1)).max(1);
}

fn parse_data_file_id(file: &str) -> Option<u64> {
    file.strip_suffix(".lrc")?.parse().ok()
}

fn resolved_record_path(root: &Path, record: &OverrideRecord) -> Result<PathBuf, LyricsStoreError> {
    match &record.location {
        StoredLocation::Sidecar { path } => Ok(path.clone()),
        StoredLocation::DataDir { file } => resolve_data_dir_file(root, file),
    }
}

fn resolve_data_dir_file(root: &Path, file: &str) -> Result<PathBuf, LyricsStoreError> {
    validate_data_file_name(file)?;
    let path = root.join(file);
    if path.parent() != Some(root) {
        return Err(LyricsStoreError::InvalidIndex(format!(
            "data file escaped lyrics dir: {file}"
        )));
    }
    Ok(path)
}

fn validate_data_file_name(file: &str) -> Result<(), LyricsStoreError> {
    let valid = !file.is_empty()
        && !file.contains('/')
        && !file.contains('\\')
        && !file.contains('\0')
        && file
            .strip_suffix(".lrc")
            .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(LyricsStoreError::InvalidIndex(format!(
            "unsafe data file name: {file}"
        )))
    }
}

fn data_file_name_from_path(path: &Path) -> Result<String, LyricsStoreError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LyricsStoreError::InvalidAudioPath)?;
    validate_data_file_name(name)?;
    Ok(name.to_owned())
}

fn validate_audio_path(path: &Path) -> Result<(), LyricsStoreError> {
    if path.as_os_str().is_empty() || path.as_os_str().to_string_lossy().contains('\0') {
        return Err(LyricsStoreError::InvalidAudioPath);
    }
    let _ = audio_parent(path)?;
    if path.file_name().is_none() {
        return Err(LyricsStoreError::InvalidAudioPath);
    }
    Ok(())
}

fn audio_parent(path: &Path) -> Result<&Path, LyricsStoreError> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent),
        _ => Err(LyricsStoreError::InvalidAudioPath),
    }
}

fn sidecar_path(audio: &Path) -> Result<PathBuf, LyricsStoreError> {
    let _ = audio_parent(audio)?;
    Ok(audio.with_extension("lrc"))
}

fn stem_collisions(audio: &Path) -> Result<Vec<PathBuf>, LyricsStoreError> {
    let parent = audio_parent(audio)?;
    let stem = audio
        .file_stem()
        .ok_or(LyricsStoreError::InvalidAudioPath)?;
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(LyricsStoreError::io("scan audio directory", err)),
    };
    let mut collisions = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| LyricsStoreError::io("scan audio directory", err))?;
        let path = entry.path();
        if path == audio {
            continue;
        }
        if path.file_stem() != Some(stem) {
            continue;
        }
        if !is_audio_path(&path) {
            continue;
        }
        collisions.push(path);
    }
    collisions.sort();
    Ok(collisions)
}

fn is_audio_path(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "aac"
            | "aif"
            | "aiff"
            | "alac"
            | "flac"
            | "m4a"
            | "mp3"
            | "mp4"
            | "ogg"
            | "opus"
            | "wav"
            | "webm"
            | "wma"
    )
}

fn official_from_record(root: &Path, record: &OverrideRecord) -> Option<Lyrics> {
    let path = resolved_record_path(root, record).ok()?;
    let text = fs::read_to_string(path).ok()?;
    Some(Lyrics {
        source: LyricsSource::ExternalFile,
        language: None,
        description: Some(provenance(record)),
        content: LyricsContent::from_text(text),
    })
}

fn provenance(record: &OverrideRecord) -> String {
    match record.source_id.as_deref() {
        Some(id) if !id.is_empty() => format!("downloaded:{}:{id}", record.provider),
        _ => format!("downloaded:{}", record.provider),
    }
}

fn atomic_write(path: &Path, bytes: &[u8], fail: bool) -> io::Result<()> {
    if fail {
        return Err(io::Error::other("injected lyrics I/O failure"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "lyrics path has no parent"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "lyrics path is not utf-8"))?;
    let temporary = parent.join(format!(".{file_name}.tmp.{}", unique_suffix()));
    if temporary.parent() != Some(parent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "temporary lyrics path escaped target directory",
        ));
    }
    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn abort_requested(abort: Option<&AtomicBool>) -> bool {
    abort.is_some_and(|flag| flag.load(Ordering::Acquire))
}

fn rollback_after_index_failure(
    final_path: &Path,
    location: LyricsSaveLocation,
    sidecar_backup: Option<PathBuf>,
) -> Result<(), LyricsStoreError> {
    match location {
        LyricsSaveLocation::Sidecar => {
            if let Some(backup) = sidecar_backup {
                restore_from_backup(&backup, final_path)
                    .map_err(|err| LyricsStoreError::io("restore original lyrics file", err))?;
            } else if final_path.exists() {
                fs::remove_file(final_path)
                    .map_err(|err| LyricsStoreError::io("remove unconfirmed lyrics file", err))?;
            }
        }
        LyricsSaveLocation::DataDir => {
            // Data-dir files are not decoder-visible. Leave the body for a retry
            // of the same id; the in-memory index has already been reverted.
        }
    }
    Ok(())
}

fn restore_from_backup(backup: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(backup, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(backup, destination)?;
            fs::remove_file(backup)?;
            Ok(())
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("lyrics.lrc");
    parent.join(format!(".{name}.bak.{}", unique_suffix()))
}

fn dir_is_marked_unwritable(faults: &IoFaults, dir: Option<&Path>) -> bool {
    let Some(dir) = dir else {
        return false;
    };
    faults
        .unwritable_dirs
        .iter()
        .any(|marked| dir == marked || dir.starts_with(marked))
}

fn is_unwritable_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

fn arc_ptr<T>(value: &Arc<T>) -> usize {
    Arc::as_ptr(value) as usize
}

fn unique_suffix() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        nanos,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
fn unique_temp_dir() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("mofu-lyrics-store-{}", unique_suffix()));
    fs::create_dir_all(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::metadata::{Lyrics, LyricsContent, LyricsLine, LyricsSource, TrackMetadata};
    use crate::player::{
        PlaybackMode, PlaybackState, PlayerSnapshot, QueuePosition, QueueSnapshot,
    };
    use std::time::Duration;

    use super::super::dto::PlayerView;

    struct Fixture {
        music: PathBuf,
        store_root: PathBuf,
        store: LyricsStore,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.music);
            let _ = fs::remove_dir_all(&self.store_root);
        }
    }

    fn fixture() -> Fixture {
        let music = unique_temp_dir().expect("music dir");
        let store_root = unique_temp_dir().expect("store dir");
        let store = LyricsStore::open(store_root.clone()).expect("open store");
        Fixture {
            music,
            store_root,
            store,
        }
    }

    fn write_audio(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"fake-audio").expect("write audio placeholder");
        path
    }

    fn request(audio: &Path, text: &str, action: LyricsConfirmAction) -> LyricsConfirmRequest {
        LyricsConfirmRequest {
            audio_path: audio.to_str().expect("utf-8 audio path").to_owned(),
            text: text.to_owned(),
            provider: "lrclib".into(),
            source_id: Some("42".into()),
            title: Some("Rubia".into()),
            artist: Some("周深".into()),
            action,
        }
    }

    fn saved(outcome: LyricsConfirmOutcome) -> LyricsSaveInfo {
        match outcome {
            LyricsConfirmOutcome::Saved(info) => info,
            other => panic!("expected saved, got {other:?}"),
        }
    }

    fn official_text(store: &LyricsStore, audio: &Path) -> String {
        let lyrics = store
            .load_official(audio.to_str().expect("utf-8"))
            .expect("official lyrics");
        match lyrics.content {
            LyricsContent::Plain { text } => text,
            LyricsContent::Timed { lines } => lines
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    fn view_for(path: &str, metadata: Option<Arc<TrackMetadata>>) -> PlayerView {
        PlayerView::from_domain(
            PlayerSnapshot {
                position: Duration::from_millis(1_200),
                duration: Duration::from_secs(200),
                state: PlaybackState::Playing,
                volume: 1.0,
                queue: QueueSnapshot {
                    main_paths: vec![path.into()],
                    up_next_paths: Vec::new(),
                    current: Some(QueuePosition::Main(0)),
                    mode: PlaybackMode::Sequential,
                },
                metadata,
            },
            None,
        )
    }

    #[test]
    fn default_save_writes_sibling_lrc_and_override() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let text = "[00:00.000]确认新词\n";
        let info = saved(
            fx.store
                .confirm(request(&audio, text, LyricsConfirmAction::Save))
                .expect("save"),
        );
        assert_eq!(info.location, LyricsSaveLocation::Sidecar);
        assert!(!info.replaced_existing);
        assert_eq!(info.saved_path, audio.with_extension("lrc"));
        assert_eq!(
            fs::read_to_string(&info.saved_path).expect("read lrc"),
            text
        );
        assert_eq!(official_text(&fx.store, &audio), "确认新词");
        let index = fs::read_to_string(fx.store_root.join(INDEX_FILE_NAME)).expect("index");
        assert!(!index.contains("确认新词"));
        assert!(index.contains("lrclib"));
        assert!(index.contains("42"));
    }

    #[test]
    fn existing_sidecar_requires_replace_and_keeps_original() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let sidecar = audio.with_extension("lrc");
        fs::write(&sidecar, "[00:00.000]原词\n").expect("existing sidecar");
        let outcome = fx
            .store
            .confirm(request(
                &audio,
                "[00:00.000]新词\n",
                LyricsConfirmAction::Save,
            ))
            .expect("confirm");
        assert!(matches!(outcome, LyricsConfirmOutcome::NeedsReplace { .. }));
        assert_eq!(
            fs::read_to_string(&sidecar).expect("read"),
            "[00:00.000]原词\n"
        );
        assert!(!fx.store.has_override(audio.to_str().unwrap()));
        assert!(!fx.store_root.join(INDEX_FILE_NAME).exists());
    }

    #[test]
    fn replace_existing_sidecar_overwrites_atomically() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let sidecar = audio.with_extension("lrc");
        fs::write(&sidecar, "[00:00.000]原词\n").expect("existing sidecar");
        let info = saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]替换词\n",
                    LyricsConfirmAction::ReplaceExistingSidecar,
                ))
                .expect("replace"),
        );
        assert!(info.replaced_existing);
        assert_eq!(info.location, LyricsSaveLocation::Sidecar);
        assert_eq!(
            fs::read_to_string(&sidecar).expect("read"),
            "[00:00.000]替换词\n"
        );
        assert_eq!(official_text(&fx.store, &audio), "替换词");
    }

    #[test]
    fn stem_collision_requires_ack_and_writes_nothing() {
        let fx = fixture();
        let flac = write_audio(&fx.music, "song.flac");
        let mp3 = write_audio(&fx.music, "song.mp3");
        let outcome = fx
            .store
            .confirm(request(&flac, "[00:00.000]词\n", LyricsConfirmAction::Save))
            .expect("confirm");
        match outcome {
            LyricsConfirmOutcome::NeedsStemCollisionAck {
                conflicting_audio,
                sidecar_path,
                ..
            } => {
                assert_eq!(conflicting_audio, vec![mp3]);
                assert_eq!(sidecar_path, flac.with_extension("lrc"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!flac.with_extension("lrc").exists());
        assert!(!fx.store.has_override(flac.to_str().unwrap()));
    }

    #[test]
    fn prefer_data_dir_after_collision_does_not_share_sidecar() {
        let fx = fixture();
        let flac = write_audio(&fx.music, "song.flac");
        let mp3 = write_audio(&fx.music, "song.mp3");
        let info = saved(
            fx.store
                .confirm(request(
                    &flac,
                    "[00:00.000]独立词\n",
                    LyricsConfirmAction::PreferDataDir,
                ))
                .expect("save to data dir"),
        );
        assert_eq!(info.location, LyricsSaveLocation::DataDir);
        assert!(info.saved_path.starts_with(fx.store.root()));
        assert!(!flac.with_extension("lrc").exists());
        assert!(fx.store.load_official(mp3.to_str().unwrap()).is_none());
        assert_eq!(official_text(&fx.store, &flac), "独立词");
        assert_ne!(
            fx.store.saved_path_for(flac.to_str().unwrap()),
            Some(flac.with_extension("lrc"))
        );
    }

    #[test]
    fn unwritable_sidecar_dir_falls_back_to_data_dir() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        fx.store.mark_unwritable(&fx.music);
        let info = saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]回退词\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("fallback"),
        );
        assert_eq!(info.location, LyricsSaveLocation::DataDir);
        assert!(!audio.with_extension("lrc").exists());
        assert!(info.saved_path.starts_with(fx.store.root()));
        assert_eq!(official_text(&fx.store, &audio), "回退词");
    }

    #[test]
    fn injected_file_write_failure_is_not_success_and_keeps_original() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let sidecar = audio.with_extension("lrc");
        fs::write(&sidecar, "[00:00.000]原词\n").expect("existing");
        fx.store.fail_next_lyrics_file_write();
        let err = fx
            .store
            .confirm(request(
                &audio,
                "[00:00.000]新词\n",
                LyricsConfirmAction::ReplaceExistingSidecar,
            ))
            .expect_err("injected failure");
        assert!(err.to_string().contains("injected"));
        assert_eq!(
            fs::read_to_string(&sidecar).expect("read"),
            "[00:00.000]原词\n"
        );
        assert!(!fx.store.has_override(audio.to_str().unwrap()));
    }

    #[test]
    fn injected_index_failure_after_new_sidecar_removes_file() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        fx.store.fail_next_index_write();
        fx.store
            .confirm(request(
                &audio,
                "[00:00.000]新词\n",
                LyricsConfirmAction::Save,
            ))
            .expect_err("index failure");
        assert!(!audio.with_extension("lrc").exists());
        assert!(!fx.store.has_override(audio.to_str().unwrap()));
        // Retry succeeds after the injected failure is consumed.
        let info = saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]新词\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("retry"),
        );
        assert_eq!(info.location, LyricsSaveLocation::Sidecar);
        assert_eq!(official_text(&fx.store, &audio), "新词");
    }

    #[test]
    fn injected_index_failure_after_replace_restores_original() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let sidecar = audio.with_extension("lrc");
        fs::write(&sidecar, "[00:00.000]原词\n").expect("existing");
        fx.store.fail_next_index_write();
        fx.store
            .confirm(request(
                &audio,
                "[00:00.000]新词\n",
                LyricsConfirmAction::ReplaceExistingSidecar,
            ))
            .expect_err("index failure");
        assert_eq!(
            fs::read_to_string(&sidecar).expect("read"),
            "[00:00.000]原词\n"
        );
        assert!(!fx.store.has_override(audio.to_str().unwrap()));
    }

    #[test]
    fn injected_index_failure_after_data_dir_write_can_retry() {
        let fx = fixture();
        let flac = write_audio(&fx.music, "song.flac");
        let _mp3 = write_audio(&fx.music, "song.mp3");
        fx.store.fail_next_index_write();
        fx.store
            .confirm(request(
                &flac,
                "[00:00.000]独立词\n",
                LyricsConfirmAction::PreferDataDir,
            ))
            .expect_err("index failure");
        assert!(!fx.store.has_override(flac.to_str().unwrap()));
        let info = saved(
            fx.store
                .confirm(request(
                    &flac,
                    "[00:00.000]独立词\n",
                    LyricsConfirmAction::PreferDataDir,
                ))
                .expect("retry"),
        );
        assert_eq!(info.location, LyricsSaveLocation::DataDir);
        assert_eq!(official_text(&fx.store, &flac), "独立词");
    }

    #[test]
    fn restart_loads_confirmed_lyrics_without_network() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]重启词\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        let reopened = LyricsStore::open(fx.store_root.clone()).expect("reopen");
        assert_eq!(official_text(&reopened, &audio), "重启词");
        let lyrics = reopened
            .load_official(audio.to_str().unwrap())
            .expect("loaded");
        assert_eq!(lyrics.source, LyricsSource::ExternalFile);
        assert_eq!(lyrics.description.as_deref(), Some("downloaded:lrclib:42"));
    }

    #[test]
    fn missing_override_target_is_not_treated_as_success() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let info = saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]词\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        fs::remove_file(&info.saved_path).expect("delete lyrics");
        assert!(fx.store.load_official(audio.to_str().unwrap()).is_none());
        fx.store.hydrate_official(audio.to_str().unwrap());
        let mut view = view_for(
            audio.to_str().unwrap(),
            Some(Arc::new(TrackMetadata {
                lyrics: vec![Lyrics {
                    source: LyricsSource::Embedded,
                    language: None,
                    description: None,
                    content: LyricsContent::Timed {
                        lines: vec![LyricsLine {
                            start_ms: 0,
                            end_ms: None,
                            text: "内嵌词".into(),
                        }],
                    },
                }],
                ..TrackMetadata::default()
            })),
        );
        fx.store.apply_official_to_view(&mut view);
        assert_eq!(
            view.metadata
                .as_ref()
                .and_then(|meta| meta.lyrics.first())
                .map(|lyrics| lyrics.source),
            Some(LyricsSource::Embedded)
        );
    }

    #[test]
    fn traversal_data_file_names_are_rejected() {
        let fx = fixture();
        for name in [
            "../secret.lrc",
            "..\\secret.lrc",
            "1/../../x.lrc",
            "foo.lrc",
        ] {
            fs::write(
                fx.store_root.join(INDEX_FILE_NAME),
                format!(
                    r#"{{"version":1,"next_id":2,"overrides":{{"/music/a.flac":{{"location":{{"kind":"data_dir","file":"{name}"}},"provider":"lrclib"}}}}}}"#
                ),
            )
            .expect("write malicious index");
            let err = LyricsStore::open(fx.store_root.clone()).expect_err("reject");
            assert!(matches!(err, LyricsStoreError::InvalidIndex(_)));
        }
    }

    #[test]
    fn apply_official_prefers_confirmed_and_leaves_other_paths_alone() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]确认稿\n[00:02.000]第二句\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        let embedded = Arc::new(TrackMetadata {
            lyrics: vec![Lyrics {
                source: LyricsSource::Embedded,
                language: None,
                description: None,
                content: LyricsContent::Timed {
                    lines: vec![LyricsLine {
                        start_ms: 0,
                        end_ms: None,
                        text: "内嵌词".into(),
                    }],
                },
            }],
            ..TrackMetadata::default()
        });
        let mut current = view_for(audio.to_str().unwrap(), Some(embedded.clone()));
        fx.store.apply_official_to_view(&mut current);
        assert_eq!(
            current
                .metadata
                .as_ref()
                .and_then(|meta| Lyrics::preferred(&meta.lyrics))
                .map(|lyrics| match &lyrics.content {
                    LyricsContent::Timed { lines } => lines[0].text.as_str(),
                    LyricsContent::Plain { text } => text.as_str(),
                }),
            Some("确认稿")
        );
        assert_eq!(current.active_lyrics_line, Some(0));
        // Embedded tags remain after the confirmed track; they are not overwritten on disk.
        assert_eq!(current.metadata.as_ref().unwrap().lyrics.len(), 2);
        assert_eq!(
            current.metadata.as_ref().unwrap().lyrics[1].source,
            LyricsSource::Embedded
        );

        let other = write_audio(&fx.music, "other.flac");
        let mut other_view = view_for(other.to_str().unwrap(), Some(embedded));
        let before_line = other_view.active_lyrics_line;
        fx.store.apply_official_to_view(&mut other_view);
        assert_eq!(other_view.active_lyrics_line, before_line);
        assert_eq!(
            other_view.metadata.as_ref().unwrap().lyrics[0].source,
            LyricsSource::Embedded
        );
    }

    #[test]
    fn apply_official_does_not_wait_on_confirm_mutex() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]确认稿\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        let holder = fx.store.clone();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _guard = holder.inner.lock().expect("hold store mutex");
            locked_tx.send(()).expect("locked");
            release_rx.recv().expect("release");
        });
        locked_rx.recv().expect("mutex held");
        let started = std::time::Instant::now();
        let mut view = view_for(
            audio.to_str().unwrap(),
            Some(Arc::new(TrackMetadata::default())),
        );
        let needed = fx.store.apply_official_to_view(&mut view);
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "apply_official_to_view waited on the confirm mutex"
        );
        assert!(needed.is_none());
        assert_eq!(
            view.metadata
                .as_ref()
                .and_then(|meta| Lyrics::preferred(&meta.lyrics))
                .map(|lyrics| match &lyrics.content {
                    LyricsContent::Timed { lines } => lines[0].text.as_str(),
                    LyricsContent::Plain { text } => text.as_str(),
                }),
            Some("确认稿")
        );
        release_tx.send(()).expect("release holder");
        handle.join().expect("holder thread");
    }

    #[test]
    fn apply_official_skips_disk_until_hydrated_after_reopen() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]确认稿\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        let reopened = LyricsStore::open(fx.store_root.clone()).expect("reopen");
        let mut view = view_for(
            audio.to_str().unwrap(),
            Some(Arc::new(TrackMetadata {
                lyrics: vec![Lyrics {
                    source: LyricsSource::Embedded,
                    language: None,
                    description: None,
                    content: LyricsContent::from_text("内嵌词".into()),
                }],
                ..TrackMetadata::default()
            })),
        );
        let needed = reopened.apply_official_to_view(&mut view);
        assert_eq!(needed.as_deref(), audio.to_str());
        assert_eq!(
            view.metadata
                .as_ref()
                .and_then(|meta| meta.lyrics.first())
                .map(|lyrics| lyrics.source),
            Some(LyricsSource::Embedded)
        );
        assert!(reopened.hydrate_official(audio.to_str().unwrap()));
        reopened.apply_official_to_view(&mut view);
        assert_eq!(
            view.metadata
                .as_ref()
                .and_then(|meta| Lyrics::preferred(&meta.lyrics))
                .map(|lyrics| match &lyrics.content {
                    LyricsContent::Timed { lines } => lines[0].text.as_str(),
                    LyricsContent::Plain { text } => text.as_str(),
                }),
            Some("确认稿")
        );
    }

    #[test]
    fn two_tracks_get_distinct_data_dir_files() {
        let fx = fixture();
        let first = write_audio(&fx.music, "a.flac");
        let second = write_audio(&fx.music, "b.flac");
        fx.store.mark_unwritable(&fx.music);
        let one = saved(
            fx.store
                .confirm(request(&first, "[00:00.000]A\n", LyricsConfirmAction::Save))
                .expect("a"),
        );
        let two = saved(
            fx.store
                .confirm(request(
                    &second,
                    "[00:00.000]B\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("b"),
        );
        assert_ne!(one.saved_path, two.saved_path);
        assert_eq!(official_text(&fx.store, &first), "A");
        assert_eq!(official_text(&fx.store, &second), "B");
    }

    #[test]
    fn abort_before_write_skips_persist() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let outcome = fx
            .store
            .confirm_with_abort(
                request(&audio, "[00:00.000]不该落盘\n", LyricsConfirmAction::Save),
                Some(&abort),
            )
            .expect("aborted confirm is not a store error");
        assert!(outcome.is_none());
        assert!(!audio.with_extension("lrc").exists());
        assert!(!fx.store.has_override(audio.to_str().unwrap()));
    }

    #[test]
    fn stale_hydrate_does_not_overwrite_remembered_official() {
        let fx = fixture();
        let audio = write_audio(&fx.music, "rubia.flac");
        saved(
            fx.store
                .confirm(request(
                    &audio,
                    "[00:00.000]确认稿\n",
                    LyricsConfirmAction::Save,
                ))
                .expect("save"),
        );
        let path = audio.to_str().unwrap();
        assert!(!fx.store.apply_stale_hydrate_for_test(path, None, 0));
        let mut view = view_for(
            path,
            Some(Arc::new(TrackMetadata {
                lyrics: vec![Lyrics {
                    source: LyricsSource::Embedded,
                    language: None,
                    description: None,
                    content: LyricsContent::from_text("内嵌词".into()),
                }],
                ..TrackMetadata::default()
            })),
        );
        assert!(fx.store.apply_official_to_view(&mut view).is_none());
        assert_eq!(
            view.metadata
                .as_ref()
                .and_then(|meta| Lyrics::preferred(&meta.lyrics))
                .map(|lyrics| match &lyrics.content {
                    LyricsContent::Timed { lines } => lines[0].text.as_str(),
                    LyricsContent::Plain { text } => text.as_str(),
                }),
            Some("确认稿")
        );
    }
    #[test]
    fn failed_data_dir_replacement_preserves_committed_body_on_reopen() {
        for action in [
            LyricsConfirmAction::Save,
            LyricsConfirmAction::PreferDataDir,
        ] {
            let fx = fixture();
            let audio = write_audio(&fx.music, "rubia.flac");
            let original = saved(
                fx.store
                    .confirm(request(
                        &audio,
                        "[00:00.000]A\n",
                        LyricsConfirmAction::PreferDataDir,
                    ))
                    .unwrap(),
            );
            fx.store.fail_next_index_write();
            fx.store
                .confirm(request(&audio, "[00:00.000]B\n", action))
                .expect_err("index write must fail");
            assert_eq!(
                fs::read_to_string(&original.saved_path).unwrap(),
                "[00:00.000]A\n"
            );
            assert_eq!(official_text(&fx.store, &audio), "A");
            let reopened = LyricsStore::open(fx.store_root.clone()).unwrap();
            assert_eq!(official_text(&reopened, &audio), "A");
            let updated = saved(
                fx.store
                    .confirm(request(&audio, "[00:00.000]B\n", action))
                    .unwrap(),
            );
            assert_ne!(original.saved_path, updated.saved_path);
            let reopened = LyricsStore::open(fx.store_root.clone()).unwrap();
            assert_eq!(official_text(&reopened, &audio), "B");
        }
    }
}
