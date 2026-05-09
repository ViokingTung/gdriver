//! Local filesystem watcher.
//!
//! Uses the `notify` crate to watch configured sync folders for changes and
//! enqueues [`SyncTask`] entries with 300 ms debounce per path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

use crate::db::queue::SyncTask;

/// Events for the same path within this window are coalesced.
const DEBOUNCE_MS: u64 = 300;

/// How often the flush tick fires.
const TICK_MS: u64 = 100;

// ─── Debounce state ──────────────────────────────────────────────────────────

struct DebounceEntry {
    first_seen: Instant,
    path: PathBuf,
    kind: EventKind,
}

// ─── Public entry point ──────────────────────────────────────────────────────

/// Run the local filesystem watcher.
///
/// Spawn as a background task.  Receives filesystem events from `notify`,
/// debounces them, converts them to [`SyncTask`] values, and sends them on
/// `task_tx` to the sync engine.
///
/// When a signal arrives on `reload_rx`, the watcher re-queries the
/// `sync_folders` table and starts watching any newly added folders.
pub async fn run(
    db: SqlitePool,
    task_tx: mpsc::Sender<SyncTask>,
    mut reload_rx: mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    // ── Load sync folder configuration ──────────────────────────────────
    let mut folder_map: HashMap<PathBuf, (String, String)> = HashMap::new();

    let (notify_tx, notify_rx) =
        std::sync::mpsc::channel::<std::result::Result<Event, notify::Error>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = notify_tx.send(res);
        },
        Config::default(),
    )?;

    // Initial load of sync folders.
    let new_folders = reload_folders(&db, &mut watcher, &mut folder_map).await;
    // Scan existing files in newly discovered folders.
    scan_existing_files(&new_folders, &task_tx).await;

    // ── Bridge blocking notify channel → async tokio channel ────────────
    let (async_tx, mut async_rx) =
        tokio::sync::mpsc::channel::<std::result::Result<Event, notify::Error>>(256);

    tokio::task::spawn_blocking(move || {
        for event in notify_rx {
            if async_tx.blocking_send(event).is_err() {
                break;
            }
        }
    });

    // ── Debounce loop ───────────────────────────────────────────────────
    let mut pending: HashMap<PathBuf, DebounceEntry> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));

    loop {
        tokio::select! {
            // ── Reload signal: re-read sync folders from DB ─────────────
            _ = reload_rx.recv() => {
                info!("watcher: reload signal received, re-reading sync folders");
                let new_folders = reload_folders(&db, &mut watcher, &mut folder_map).await;
                scan_existing_files(&new_folders, &task_tx).await;
            }

            event = async_rx.recv() => {
                match event {
                    Some(Ok(event)) => {
                        for path in &event.paths {
                            let key = canonicalise_key(path);
                            trace!(?key, ?event.kind, "fs event");
                            match pending.get_mut(&key) {
                                Some(entry) => {
                                    entry.kind = event.kind.clone();
                                }
                                None => {
                                    pending.insert(key.clone(), DebounceEntry {
                                        first_seen: Instant::now(),
                                        path: key.clone(),
                                        kind: event.kind.clone(),
                                    });
                                }
                            }
                        }
                    }
                    Some(Err(e)) => warn!("notify error: {}", e),
                    None => {
                        info!("notify event stream closed; watcher exiting");
                        break;
                    }
                }
            }

            _ = tick.tick() => {
                let now = Instant::now();
                let threshold = Duration::from_millis(DEBOUNCE_MS);
                let mut tasks: Vec<SyncTask> = Vec::new();

                pending.retain(|_key, entry| {
                    if now.duration_since(entry.first_seen) >= threshold {
                        if let Some(task) = event_to_task(
                            &entry.path,
                            &entry.kind,
                            &folder_map,
                        ) {
                            tasks.push(task);
                        }
                        false // remove from pending
                    } else {
                        true // still debouncing
                    }
                });

                for task in tasks {
                    if let Err(e) = task_tx.try_send(task) {
                        warn!("watcher → sync engine channel full; task dropped: {}", e);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Re-read enabled sync folders from the database and start watching any
/// new directories.  Already-watched directories are left untouched.
///
/// Returns (path, folder_type, account_id) for newly added folders so the
/// caller can scan them for existing files.
async fn reload_folders(
    db: &SqlitePool,
    watcher: &mut RecommendedWatcher,
    folder_map: &mut HashMap<PathBuf, (String, String)>,
) -> Vec<(PathBuf, String, String)> {
    let folders = match crate::db::sync_folders::list_enabled(db).await {
        Ok(f) => f,
        Err(e) => {
            warn!("watcher: failed to reload sync folders: {e:#}");
            return Vec::new();
        }
    };

    let mut new_folders: Vec<(PathBuf, String, String)> = Vec::new();

    for folder in &folders {
        let path = Path::new(&folder.local_path);
        if folder_map.contains_key(&path.to_path_buf()) {
            continue; // already watching
        }
        if path.is_dir() {
            match watcher.watch(path, RecursiveMode::Recursive) {
                Ok(()) => {
                    info!("watching {}", path.display());
                    folder_map.insert(
                        path.to_path_buf(),
                        (folder.account_id.clone(), folder.folder_type.clone()),
                    );
                    new_folders.push((
                        path.to_path_buf(),
                        folder.folder_type.clone(),
                        folder.account_id.clone(),
                    ));
                }
                Err(e) => warn!("failed to watch {}: {}", path.display(), e),
            }
        } else {
            debug!("sync folder does not exist yet, skipping: {}", path.display());
        }
    }

    if folder_map.is_empty() {
        info!("no directories to watch; watcher idle");
    } else {
        debug!("watching {} sync folder(s)", folder_map.len());
    }

    new_folders
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Normalise a path for use as a deduplication key.
fn canonicalise_key(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Convert a single filesystem event to an optional [`SyncTask`].
///
/// Returns `None` when the event should be ignored (e.g. directory-only changes,
/// metadata-only events, or paths that don't belong to any watched folder).
fn event_to_task(
    path: &Path,
    kind: &EventKind,
    folder_map: &HashMap<PathBuf, (String, String)>,
) -> Option<SyncTask> {
    use notify::event::{ModifyKind, RemoveKind};

    // Only care about regular files — directory operations are irrelevant.
    if path.is_dir() {
        return None;
    }

    // Determine which sync folder this path belongs to.
    let (account_id, folder_type) = find_owning_folder(path, folder_map)?;

    // Photos folders use the Google Photos backup flow; drive folders use the
    // normal Drive upload flow.
    let is_photos = folder_type == "photos";

    let operation = match kind {
        EventKind::Create(_) => {
            if is_photos { "photos_backup" } else { "upload" }
        }
        EventKind::Modify(modify_kind) => match modify_kind {
            ModifyKind::Name(_) => {
                if is_photos { "photos_backup" } else { "upload" }
            }
            ModifyKind::Data(_) | ModifyKind::Any => {
                if is_photos { "photos_backup" } else { "upload" }
            }
            ModifyKind::Metadata(_) => {
                // Metadata-only changes (e.g. permissions) are not interesting.
                return None;
            }
            _ => {
                if is_photos { "photos_backup" } else { "upload" }
            }
        },
        EventKind::Remove(remove_kind) => match remove_kind {
            RemoveKind::File | RemoveKind::Any => "delete",
            RemoveKind::Other => "delete",
            _ => return None,
        },
        _ => return None, // Access, Other — ignore
    };

    let now = chrono::Utc::now().timestamp_millis();
    let local_path = path.to_string_lossy().to_string();

    Some(SyncTask {
        id: None,
        account_id: account_id.to_string(),
        file_id: None, // filled by sync engine after processing
        operation: operation.to_string(),
        local_path: Some(local_path),
        priority: 5,
        status: "pending".to_string(),
        retry_count: 0,
        error_msg: None,
        created_at: now,
        updated_at: now,
    })
}

/// Find the watched folder that owns the given path (longest prefix match).
fn find_owning_folder<'a>(
    path: &Path,
    folder_map: &'a HashMap<PathBuf, (String, String)>,
) -> Option<&'a (String, String)> {
    let canonical = canonicalise_key(path);
    folder_map
        .iter()
        .filter(|(root, _)| canonical.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, info)| info)
}

/// Walk newly added sync folders and enqueue upload tasks for every existing
/// regular file.  This handles the case where files already exist in the folder
/// before the watcher starts — `notify` only emits events for *new* changes.
async fn scan_existing_files(
    folders: &[(PathBuf, String, String)],
    task_tx: &mpsc::Sender<SyncTask>,
) {
    let now = chrono::Utc::now().timestamp_millis();

    for (root, folder_type, account_id) in folders {
        let is_photos = folder_type == "photos";
        let operation = if is_photos { "photos_backup" } else { "upload" };

        info!("scanning existing files in {}", root.display());
        let mut count: u64 = 0;

        walk_and_enqueue(root, account_id, operation, now, task_tx, &mut count).await;

        info!(
            "scan complete for {}: {} file(s) enqueued",
            root.display(),
            count,
        );
    }
}

/// Recursively walk a directory and send upload tasks for every regular file.
///
/// Uses `send().await` for backpressure so we don't drop tasks when the
/// channel is full — the walk pauses until the sync engine drains a slot.
async fn walk_and_enqueue(
    dir: &Path,
    account_id: &str,
    operation: &str,
    now: i64,
    task_tx: &mpsc::Sender<SyncTask>,
    count: &mut u64,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("scan: cannot read {}: {e}", dir.display());
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(walk_and_enqueue(&path, account_id, operation, now, task_tx, count)).await;
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let local_path = path.to_string_lossy().to_string();
        let task = SyncTask {
            id: None,
            account_id: account_id.to_string(),
            file_id: None,
            operation: operation.to_string(),
            local_path: Some(local_path),
            priority: 5,
            status: "pending".to_string(),
            retry_count: 0,
            error_msg: None,
            created_at: now,
            updated_at: now,
        };

        if let Err(e) = task_tx.send(task).await {
            warn!("scan: channel closed, stopping: {e}");
            return;
        }
        *count += 1;
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use std::fs;
    use tempfile::TempDir;

    fn folder_map(root: &Path, account_id: &str) -> HashMap<PathBuf, (String, String)> {
        let mut m = HashMap::new();
        m.insert(root.to_path_buf(), (account_id.to_string(), "drive".to_string()));
        m
    }

    fn photos_folder_map(root: &Path, account_id: &str) -> HashMap<PathBuf, (String, String)> {
        let mut m = HashMap::new();
        m.insert(root.to_path_buf(), (account_id.to_string(), "photos".to_string()));
        m
    }

    // ── event_to_task ─────────────────────────────────────────────────────

    #[test]
    fn create_event_emits_upload() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("new.txt");
        fs::write(&file, b"hello").unwrap();

        let map = folder_map(tmp.path(), "acct-1");
        let task = event_to_task(&file, &EventKind::Create(CreateKind::File), &map).unwrap();

        assert_eq!(task.operation, "upload");
        assert_eq!(task.account_id, "acct-1");
        assert_eq!(task.local_path.unwrap(), file.to_string_lossy());
        assert_eq!(task.status, "pending");
        assert_eq!(task.priority, 5);
    }

    #[test]
    fn modify_data_event_emits_upload() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("changed.txt");
        fs::write(&file, b"data").unwrap();

        let map = folder_map(tmp.path(), "acct-2");
        let task = event_to_task(&file, &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)), &map).unwrap();

        assert_eq!(task.operation, "upload");
        assert_eq!(task.account_id, "acct-2");
    }

    #[test]
    fn remove_file_event_emits_delete() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("gone.txt");
        fs::write(&file, b"temp").unwrap();

        let map = folder_map(tmp.path(), "acct-1");
        let task = event_to_task(&file, &EventKind::Remove(RemoveKind::File), &map).unwrap();

        assert_eq!(task.operation, "delete");
    }

    #[test]
    fn directory_events_are_ignored() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("subdir");
        fs::create_dir(&dir).unwrap();

        let map = folder_map(tmp.path(), "acct-1");
        assert!(event_to_task(&dir, &EventKind::Create(CreateKind::Folder), &map).is_none());
    }

    #[test]
    fn metadata_only_events_are_ignored() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("meta.txt");
        fs::write(&file, b"data").unwrap();

        let map = folder_map(tmp.path(), "acct-1");
        assert!(
            event_to_task(&file, &EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::Any)), &map)
                .is_none()
        );
    }

    #[test]
    fn path_outside_watched_folder_returns_none() {
        let tmp = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let file = other.path().join("outside.txt");
        fs::write(&file, b"data").unwrap();

        let map = folder_map(tmp.path(), "acct-1");
        assert!(event_to_task(&file, &EventKind::Create(CreateKind::File), &map).is_none());
    }

    // ── find_owning_folder ───────────────────────────────────────────────

    #[test]
    fn finds_nested_path() {
        let map = folder_map(Path::new("/sync"), "acct-1");
        let child = Path::new("/sync/sub/file.txt");
        let (acct, ftype) = find_owning_folder(child, &map).unwrap();
        assert_eq!(acct, "acct-1");
        assert_eq!(ftype, "drive");
    }

    #[test]
    fn exact_root_match() {
        let map = folder_map(Path::new("/sync"), "acct-1");
        let (acct, _) = find_owning_folder(Path::new("/sync"), &map).unwrap();
        assert_eq!(acct, "acct-1");
    }

    #[test]
    fn deepest_prefix_wins() {
        let mut map = HashMap::new();
        map.insert(PathBuf::from("/sync"), ("acct-1".into(), "drive".into()));
        map.insert(
            PathBuf::from("/sync/sub"),
            ("acct-2".into(), "photos".into()),
        );

        let (acct, ftype) = find_owning_folder(Path::new("/sync/sub/deep/file.txt"), &map).unwrap();
        assert_eq!(acct, "acct-2");
        assert_eq!(ftype, "photos");
    }

    #[test]
    fn no_match_returns_none() {
        let map = folder_map(Path::new("/sync"), "acct-1");
        assert!(find_owning_folder(Path::new("/other/file.txt"), &map).is_none());
    }

    // ── photos folder → photos_backup operation ──────────────────────────

    #[test]
    fn photos_folder_create_emits_photos_backup() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("photo.jpg");
        fs::write(&file, b"fake-jpeg").unwrap();

        let map = photos_folder_map(tmp.path(), "acct-1");
        let task = event_to_task(&file, &EventKind::Create(CreateKind::File), &map).unwrap();

        assert_eq!(task.operation, "photos_backup");
        assert_eq!(task.account_id, "acct-1");
    }

    #[test]
    fn photos_folder_modify_data_emits_photos_backup() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("photo.png");
        fs::write(&file, b"fake-png").unwrap();

        let map = photos_folder_map(tmp.path(), "acct-1");
        let task = event_to_task(&file, &EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)), &map).unwrap();

        assert_eq!(task.operation, "photos_backup");
    }

    #[test]
    fn photos_folder_remove_emits_delete() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("old.jpg");
        fs::write(&file, b"data").unwrap();

        let map = photos_folder_map(tmp.path(), "acct-1");
        let task = event_to_task(&file, &EventKind::Remove(RemoveKind::File), &map).unwrap();

        assert_eq!(task.operation, "delete");
    }

    #[test]
    fn photos_folder_metadata_only_still_ignored() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("photo.jpg");
        fs::write(&file, b"data").unwrap();

        let map = photos_folder_map(tmp.path(), "acct-1");
        assert!(
            event_to_task(&file, &EventKind::Modify(ModifyKind::Metadata(notify::event::MetadataKind::Any)), &map)
                .is_none()
        );
    }
}
