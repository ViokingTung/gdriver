// ─── Windows WinFSP Virtual Filesystem ────────────────────────────────────
//
// Implements the WinFSP `FileSystem` trait to expose Google Drive as a
// virtual drive letter (default G:\). Structurally mirrors the Linux FUSE
// implementation in `linux.rs` but adapted for the WinFSP API surface.
//
// Key differences from FUSE:
//   - WinFSP uses NTSTATUS error codes
//   - File attributes use Windows FILE_ATTRIBUTE_* constants
//   - Security descriptors are required (we use permissive defaults)
//   - File names are Unicode (UTF-16) via `&OsStr`
//   - The filesystem is mounted as a drive letter via `FileSystemHost`

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use tracing::{info, warn};
use winfsp::{
    error::{Result as WinResult, NTSTATUS},
    filesystem::{
        FileInfo, FileSystem, FileSystemHost, OpenFileInfo, SecurityDescriptor, VolumeInfo,
        FILE_ATTRIBUTE, FSP_FSCTL_VOLUME_PARAMS,
    },
    interface::FileSystemContext,
};

use crate::{
    backend::{VfsBackend, VfsContext, VfsHandle},
    db,
};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Maximum time to wait for an on-demand download before returning an error.
const DOWNLOAD_TIMEOUT_MS: u64 = 30_000;

/// Default volume label for the virtual drive.
const VOLUME_LABEL: &str = "Google Drive";

/// Sector size reported to Windows.
const SECTOR_SIZE: u16 = 512;

/// Sectors per allocation unit.
const SECTORS_PER_ALLOCATION_UNIT: u16 = 1;

// ─── Open file handle ──────────────────────────────────────────────────────

/// Tracks an open file for the lifetime between `open` and `close`.
#[derive(Debug)]
struct OpenHandle {
    /// Inode (rowid) of the file.
    ino: u64,
    /// Local cache path for reads/writes.
    local_path: PathBuf,
    /// Whether the file was opened for writing.
    writable: bool,
}

// ─── WinFspVfs ──────────────────────────────────────────────────────────────

/// WinFSP filesystem implementation backed by Google Drive.
///
/// Holds the VFS context and a map of open file handles. Implements the
/// WinFSP `FileSystem` trait so it can be mounted as a virtual drive.
pub struct WinFspVfs {
    /// VFS context (cache directory, mount point, DB pool).
    ctx: VfsContext,
    /// Open file handles protected by a mutex (WinFSP callbacks may arrive
    /// from multiple threads concurrently).
    open_files: Arc<Mutex<HashMap<u64, OpenHandle>>>,
    /// Monotonically increasing file handle counter.
    next_fh: Arc<Mutex<u64>>,
}

impl WinFspVfs {
    /// Create a new WinFSP filesystem instance.
    pub fn new(ctx: VfsContext) -> Self {
        Self {
            ctx,
            open_files: Arc::new(Mutex::new(HashMap::new())),
            next_fh: Arc::new(Mutex::new(1)),
        }
    }

    /// Execute an async future synchronously using the current Tokio runtime.
    ///
    /// WinFSP callbacks are synchronous; this bridges to the async `sqlx` queries.
    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }

    /// Allocate a new file handle number.
    fn alloc_fh(&self) -> u64 {
        let mut fh = self.next_fh.lock().unwrap();
        let val = *fh;
        *fh += 1;
        val
    }

    /// Insert an open handle and return its file handle number.
    fn insert_handle(&self, handle: OpenHandle) -> u64 {
        let fh = self.alloc_fh();
        self.open_files.lock().unwrap().insert(fh, handle);
        fh
    }

    /// Remove and return an open handle by file handle number.
    fn remove_handle(&self, fh: u64) -> Option<OpenHandle> {
        self.open_files.lock().unwrap().remove(&fh)
    }

    /// Look up an open handle by file handle number.
    fn get_handle(&self, fh: u64) -> Option<OpenHandle> {
        self.open_files.lock().unwrap().get(&fh).cloned()
    }

    /// Build the local cache path for a file.
    fn cache_path(&self, account_id: &str, file_id: &str) -> PathBuf {
        self.ctx.cache_dir.join(account_id).join(file_id)
    }

    /// Convert a [`db::FileMeta`] into WinFSP [`FileInfo`].
    fn file_to_info(meta: &db::FileMeta) -> FileInfo {
        let is_dir = meta.mime_type == "application/vnd.google-apps.folder";
        let attr = if is_dir {
            FILE_ATTRIBUTE::DIRECTORY
        } else {
            FILE_ATTRIBUTE::NORMAL
        };

        let mtime = UNIX_EPOCH + Duration::from_millis(meta.modified_time as u64);
        let now = SystemTime::now();

        FileInfo {
            file_attributes: attr,
            reparse_tag: 0,
            allocation_size: if is_dir { 0 } else { meta.size },
            file_size: if is_dir { 0 } else { meta.size },
            creation_time: mtime,
            last_access_time: now,
            last_write_time: mtime,
            change_time: mtime,
            index_number: meta.inode,
            hard_links: 0,
            ea_size: 0,
        }
    }

    /// Build `FileInfo` for the synthetic root directory (inode 1).
    fn root_info() -> FileInfo {
        let now = SystemTime::now();
        FileInfo {
            file_attributes: FILE_ATTRIBUTE::DIRECTORY,
            reparse_tag: 0,
            allocation_size: 0,
            file_size: 0,
            creation_time: now,
            last_access_time: now,
            last_write_time: now,
            change_time: now,
            index_number: db::ROOT_INODE,
            hard_links: 0,
            ea_size: 0,
        }
    }
}

// Implement Clone so WinFSP can share the context across threads.
impl Clone for WinFspVfs {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx.clone(),
            open_files: Arc::clone(&self.open_files),
            next_fh: Arc::clone(&self.next_fh),
        }
    }
}

// ─── Helper: check if MIME is Google Workspace ──────────────────────────────

fn is_google_workspace_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "application/vnd.google-apps.document"
            | "application/vnd.google-apps.spreadsheet"
            | "application/vnd.google-apps.presentation"
            | "application/vnd.google-apps.drawing"
            | "application/vnd.google-apps.script"
    )
}

/// Guess a MIME type from a file name extension.
fn guess_mime_from_name(name: &str) -> String {
    match std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
    {
        Some("txt") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("zip") => "application/zip",
        Some("gz" | "tar") => "application/gzip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ─── WinFSP FileSystem implementation ───────────────────────────────────────

impl FileSystemContext for WinFspVfs {
    type FileContext = OpenHandle;

    fn get_volume_info(&self) -> WinResult<VolumeInfo> {
        Ok(VolumeInfo {
            total_size: 1024 * 1024 * 1024 * 1024, // 1 TB virtual
            free_size: 512 * 1024 * 1024 * 1024,   // 512 GB virtual
            volume_label: VOLUME_LABEL.to_string(),
        })
    }

    fn get_security_by_name(
        &self,
        file_name: &OsStr,
        _find_reparse_point: Option<fn() -> bool>,
        security_descriptor: &mut [u8],
    ) -> WinResult<(u32, u32)> {
        let name_str = file_name.to_string_lossy();

        // Root directory
        if name_str.is_empty() || name_str == "\\" || name_str == "/" {
            return Ok((FILE_ATTRIBUTE::DIRECTORY.bits(), 0));
        }

        // Strip leading backslash
        let name_clean = name_str.trim_start_matches('\\').trim_start_matches('/');

        // Parse path components and walk the tree
        let components: Vec<&str> = name_clean.split('\\').collect();
        let mut current_inode = db::ROOT_INODE;

        for (i, component) in components.iter().enumerate() {
            let meta = match self.block_on(db::lookup_by_parent_and_name(
                &self.ctx.db,
                current_inode,
                component,
            )) {
                Ok(Some(m)) => m,
                Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            };

            if i == components.len() - 1 {
                // Last component — return its attributes
                let is_dir = meta.mime_type == "application/vnd.google-apps.folder";
                let attr = if is_dir {
                    FILE_ATTRIBUTE::DIRECTORY
                } else {
                    FILE_ATTRIBUTE::NORMAL
                };
                return Ok((attr.bits(), 0));
            }

            // Not last component — must be a directory
            if meta.mime_type != "application/vnd.google-apps.folder" {
                return Err(NTSTATUS::STATUS_NOT_A_DIRECTORY);
            }
            current_inode = meta.inode;
        }

        Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND)
    }

    fn open(
        &self,
        file_name: &OsStr,
        create_options: u32,
        granted_access: u32,
        _file_info: &mut FileInfo,
    ) -> WinResult<Self::FileContext> {
        let name_str = file_name.to_string_lossy();

        // Parse the path to find the inode
        let name_clean = name_str.trim_start_matches('\\').trim_start_matches('/');
        let components: Vec<&str> = if name_clean.is_empty() {
            vec![]
        } else {
            name_clean.split('\\').collect()
        };

        let mut current_inode = db::ROOT_INODE;
        for component in &components {
            let meta = match self.block_on(db::lookup_by_parent_and_name(
                &self.ctx.db,
                current_inode,
                component,
            )) {
                Ok(Some(m)) => m,
                Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            };
            current_inode = meta.inode;
        }

        // Root directory — return a placeholder handle
        if components.is_empty() {
            return Ok(OpenHandle {
                ino: db::ROOT_INODE,
                local_path: PathBuf::new(),
                writable: false,
            });
        }

        let details =
            match self.block_on(db::get_file_details_by_inode(&self.ctx.db, current_inode)) {
                Ok(Some(d)) => d,
                Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            };

        // Cannot open Google Workspace files directly
        if is_google_workspace_mime(&details.mime_type) {
            return Err(NTSTATUS::STATUS_NOT_SUPPORTED);
        }

        let local_path = details
            .local_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cache_path(&details.account_id, &details.file_id));

        let writable = (granted_access & 0x00000002) != 0; // FILE_WRITE_DATA

        Ok(OpenHandle {
            ino: current_inode,
            local_path,
            writable,
        })
    }

    fn close(&self, file_context: Self::FileContext) {
        // Drop the handle — no further cleanup needed here.
        // Upload tasks are enqueued during `flush` or `cleanup`.
        drop(file_context);
    }

    fn get_file_info(
        &self,
        file_context: &Self::FileContext,
        file_info: &mut FileInfo,
    ) -> WinResult<()> {
        if file_context.ino == db::ROOT_INODE {
            *file_info = Self::root_info();
            return Ok(());
        }

        let meta = match self.block_on(db::get_file_by_inode(&self.ctx.db, file_context.ino)) {
            Ok(Some(m)) => m,
            Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
            Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
        };

        *file_info = Self::file_to_info(&meta);
        Ok(())
    }

    fn read_directory(
        &self,
        file_context: &Self::FileContext,
        _pattern: Option<&OsStr>,
        marker: Option<&OsStr>,
        mut buffer: &mut [u8],
    ) -> WinResult<usize> {
        let children =
            match self.block_on(db::list_children_by_inode(&self.ctx.db, file_context.ino)) {
                Ok(c) => c,
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            };

        let mut bytes_written = 0;

        for child in &children {
            // Skip entries before the marker
            if let Some(m) = marker {
                if child.name <= m.to_string_lossy().to_string() {
                    continue;
                }
            }

            let info = Self::file_to_info(child);
            let name = &child.name;

            // Try to add the entry to the buffer
            match winfsp::filesystem::write_file_info(&mut buffer, &info, name) {
                Ok(written) => bytes_written += written,
                Err(_) => break, // Buffer full
            }
        }

        Ok(bytes_written)
    }

    fn read(
        &self,
        file_context: &Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> WinResult<u32> {
        // Cannot read the root directory
        if file_context.ino == db::ROOT_INODE {
            return Err(NTSTATUS::STATUS_INVALID_DEVICE_REQUEST);
        }

        let details = match self.block_on(db::get_file_details_by_inode(
            &self.ctx.db,
            file_context.ino,
        )) {
            Ok(Some(d)) => d,
            Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
            Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
        };

        let local_path = details
            .local_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cache_path(&details.account_id, &details.file_id));

        // If the file is cloud_only, trigger an on-demand download.
        if details.sync_state == "cloud_only" || !local_path.exists() {
            // Ensure the parent directory exists.
            if let Some(parent) = local_path.parent() {
                if !parent.exists() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        tracing::error!("read: failed to create cache dir: {e:#}");
                        return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
                    }
                }
            }

            // Enqueue a download task.
            if let Err(e) = self.block_on(db::enqueue_download_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                &local_path.to_string_lossy(),
            )) {
                tracing::error!("read: failed to enqueue download: {e:#}");
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }

            // Block until download completes or timeout.
            match self.block_on(db::wait_for_download(
                &self.ctx.db,
                &details.file_id,
                &details.account_id,
                DOWNLOAD_TIMEOUT_MS,
            )) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("read: download timed out or failed");
                    return Err(NTSTATUS::STATUS_IO_TIMEOUT);
                }
                Err(e) => {
                    tracing::error!("read: download wait error: {e:#}");
                    return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
                }
            }
        }

        // Read from the local file.
        let mut file = match fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "read: failed to open local file {}: {e:#}",
                    local_path.display()
                );
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset)) {
            tracing::error!("read: seek to {offset} failed: {e:#}");
            return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
        }

        match file.read(buffer) {
            Ok(n) => Ok(n as u32),
            Err(e) => {
                tracing::error!("read: read failed: {e:#}");
                Err(NTSTATUS::STATUS_INTERNAL_ERROR)
            }
        }
    }

    fn write(
        &self,
        file_context: &mut Self::FileContext,
        buffer: &[u8],
        offset: u64,
        _write_to_eof: bool,
        _constrained_io: bool,
    ) -> WinResult<u32> {
        if file_context.ino == db::ROOT_INODE {
            return Err(NTSTATUS::STATUS_INVALID_DEVICE_REQUEST);
        }

        // Open the local file for writing.
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&file_context.local_path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("write: failed to open local file: {e:#}");
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset)) {
            tracing::error!("write: seek to {offset} failed: {e:#}");
            return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
        }

        match std::io::Write::write(&mut file, buffer) {
            Ok(n) => {
                // Update the cached size and mtime in the DB.
                let new_end = offset as i64 + n as i64;
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let _ = self.block_on(db::update_file_size_mtime(
                    &self.ctx.db,
                    file_context.ino,
                    new_end,
                    now_ms,
                ));

                Ok(n as u32)
            }
            Err(e) => {
                tracing::error!("write: write failed: {e:#}");
                Err(NTSTATUS::STATUS_INTERNAL_ERROR)
            }
        }
    }

    fn flush(&self, file_context: &Self::FileContext) -> WinResult<()> {
        if file_context.ino == db::ROOT_INODE {
            return Ok(());
        }

        let details = match self.block_on(db::get_file_details_by_inode(
            &self.ctx.db,
            file_context.ino,
        )) {
            Ok(Some(d)) => d,
            _ => return Ok(()),
        };

        // Enqueue upload for locally-modified files.
        if details.sync_state == "modified" {
            if let Err(e) = self.block_on(db::enqueue_upload_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                &file_context.local_path.to_string_lossy(),
            )) {
                tracing::error!("flush: enqueue_upload failed: {e:#}");
            }
        }

        Ok(())
    }

    fn cleanup(&self, file_context: &Self::FileContext, _file_name: Option<&OsStr>, flags: u32) {
        // FspCleanupDelete: file is being deleted
        if flags & 0x01 != 0 {
            if file_context.ino != db::ROOT_INODE {
                if let Ok(Some(details)) = self.block_on(db::get_file_details_by_inode(
                    &self.ctx.db,
                    file_context.ino,
                )) {
                    // Enqueue a delete task for the sync engine.
                    let _ = self.block_on(db::enqueue_task(
                        &self.ctx.db,
                        &details.account_id,
                        &details.file_id,
                        "delete",
                        details.local_path.as_deref(),
                    ));

                    // Remove the local cache file if present.
                    if let Some(ref local_path) = details.local_path {
                        let _ = fs::remove_file(local_path);
                    }
                }

                let _ = self.block_on(db::soft_delete_by_inode(&self.ctx.db, file_context.ino));
            }
        }
    }

    fn create(
        &self,
        file_name: &OsStr,
        create_options: u32,
        granted_access: u32,
        file_attributes: u32,
        _security_descriptor: Option<&[u8]>,
        allocation_size: u64,
        file_info: &mut FileInfo,
    ) -> WinResult<Self::FileContext> {
        let name_str = file_name.to_string_lossy().to_string();
        let name_clean = name_str.trim_start_matches('\\').trim_start_matches('/');

        // Split into parent path and new name
        let (parent_path, new_name) = match name_clean.rfind('\\') {
            Some(pos) => (&name_clean[..pos], &name_clean[pos + 1..]),
            None => ("", name_clean),
        };

        // Walk to the parent directory
        let mut parent_inode = db::ROOT_INODE;
        if !parent_path.is_empty() {
            for component in parent_path.split('\\') {
                let meta = match self.block_on(db::lookup_by_parent_and_name(
                    &self.ctx.db,
                    parent_inode,
                    component,
                )) {
                    Ok(Some(m)) => m,
                    Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_PATH_NOT_FOUND),
                    Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
                };

                if meta.mime_type != "application/vnd.google-apps.folder" {
                    return Err(NTSTATUS::STATUS_NOT_A_DIRECTORY);
                }
                parent_inode = meta.inode;
            }
        }

        // Check if a file with this name already exists
        if let Ok(Some(_)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent_inode,
            new_name,
        )) {
            return Err(NTSTATUS::STATUS_OBJECT_NAME_COLLISION);
        }

        // Determine if this is a directory creation
        let is_directory = create_options & 0x00000001 != 0; // FILE_DIRECTORY_FILE

        // Get parent account_id and file_id
        let (account_id, parent_file_id) = if parent_inode == db::ROOT_INODE {
            match self.block_on(db::get_first_account_id(&self.ctx.db)) {
                Ok(Some(acct)) => (acct, None),
                Ok(None) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            }
        } else {
            match self.block_on(db::get_parent_info(&self.ctx.db, parent_inode)) {
                Ok(Some(info)) => info,
                _ => return Err(NTSTATUS::STATUS_OBJECT_PATH_NOT_FOUND),
            }
        };

        // Generate a temporary file ID
        let temp_file_id = format!(
            "local-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        if is_directory {
            // Create a folder
            let ino = match self.block_on(db::insert_local_folder(
                &self.ctx.db,
                &temp_file_id,
                &account_id,
                new_name,
                parent_file_id.as_deref(),
            )) {
                Ok(ino) => ino,
                Err(e) => {
                    tracing::error!("create: insert folder failed: {e:#}");
                    return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
                }
            };

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;

            *file_info = FileInfo {
                file_attributes: FILE_ATTRIBUTE::DIRECTORY,
                reparse_tag: 0,
                allocation_size: 0,
                file_size: 0,
                creation_time: UNIX_EPOCH + Duration::from_millis(now_ms as u64),
                last_access_time: UNIX_EPOCH + Duration::from_millis(now_ms as u64),
                last_write_time: UNIX_EPOCH + Duration::from_millis(now_ms as u64),
                change_time: UNIX_EPOCH + Duration::from_millis(now_ms as u64),
                index_number: ino,
                hard_links: 0,
                ea_size: 0,
            };

            Ok(OpenHandle {
                ino,
                local_path: PathBuf::new(),
                writable: false,
            })
        } else {
            // Create a file
            let mime_type = guess_mime_from_name(new_name);
            let local_path = self.cache_path(&account_id, &temp_file_id);

            // Ensure the cache directory exists.
            if let Some(parent_dir) = local_path.parent() {
                if !parent_dir.exists() {
                    if let Err(e) = fs::create_dir_all(parent_dir) {
                        tracing::error!("create: mkdir {:?} failed: {e:#}", parent_dir);
                        return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
                    }
                }
            }

            // Insert a row into drive_files.
            let ino = match self.block_on(db::insert_local_file(
                &self.ctx.db,
                &temp_file_id,
                &account_id,
                new_name,
                &mime_type,
                parent_file_id.as_deref(),
                &local_path.to_string_lossy(),
            )) {
                Ok(ino) => ino,
                Err(e) => {
                    tracing::error!("create: insert_local_file failed: {e:#}");
                    return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
                }
            };

            // Create an empty file on disk.
            if let Err(e) = fs::File::create(&local_path) {
                tracing::error!(
                    "create: failed to create file {}: {e:#}",
                    local_path.display()
                );
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }

            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            *file_info = FileInfo {
                file_attributes: FILE_ATTRIBUTE::NORMAL,
                reparse_tag: 0,
                allocation_size: 0,
                file_size: 0,
                creation_time: UNIX_EPOCH + Duration::from_millis(now_ms),
                last_access_time: UNIX_EPOCH + Duration::from_millis(now_ms),
                last_write_time: UNIX_EPOCH + Duration::from_millis(now_ms),
                change_time: UNIX_EPOCH + Duration::from_millis(now_ms),
                index_number: ino,
                hard_links: 0,
                ea_size: 0,
            };

            Ok(OpenHandle {
                ino,
                local_path,
                writable: true,
            })
        }
    }

    fn rename(
        &self,
        file_context: &Self::FileContext,
        file_name: &OsStr,
        new_file_name: &OsStr,
        replace_if_exists: bool,
    ) -> WinResult<()> {
        let old_name = file_name.to_string_lossy().to_string();
        let new_name = new_file_name.to_string_lossy().to_string();

        let old_clean = old_name.trim_start_matches('\\').trim_start_matches('/');
        let new_clean = new_name.trim_start_matches('\\').trim_start_matches('/');

        // Parse old path
        let (old_parent_path, old_file_name) = match old_clean.rfind('\\') {
            Some(pos) => (&old_clean[..pos], &old_clean[pos + 1..]),
            None => ("", old_clean),
        };

        // Parse new path
        let (new_parent_path, new_file_name) = match new_clean.rfind('\\') {
            Some(pos) => (&new_clean[..pos], &new_clean[pos + 1..]),
            None => ("", new_clean),
        };

        // Walk to old parent
        let mut old_parent_inode = db::ROOT_INODE;
        if !old_parent_path.is_empty() {
            for component in old_parent_path.split('\\') {
                let meta = match self.block_on(db::lookup_by_parent_and_name(
                    &self.ctx.db,
                    old_parent_inode,
                    component,
                )) {
                    Ok(Some(m)) => m,
                    Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_PATH_NOT_FOUND),
                    Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
                };
                old_parent_inode = meta.inode;
            }
        }

        // Walk to new parent
        let mut new_parent_inode = db::ROOT_INODE;
        if !new_parent_path.is_empty() {
            for component in new_parent_path.split('\\') {
                let meta = match self.block_on(db::lookup_by_parent_and_name(
                    &self.ctx.db,
                    new_parent_inode,
                    component,
                )) {
                    Ok(Some(m)) => m,
                    Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_PATH_NOT_FOUND),
                    Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
                };
                new_parent_inode = meta.inode;
            }
        }

        // Look up the source file
        let meta = match self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            old_parent_inode,
            old_file_name,
        )) {
            Ok(Some(m)) => m,
            Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
            Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
        };

        // Check if target already exists
        if let Ok(Some(target_meta)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            new_parent_inode,
            new_file_name,
        )) {
            if !replace_if_exists {
                return Err(NTSTATUS::STATUS_OBJECT_NAME_COLLISION);
            }
            // Soft-delete the conflicting target so the rename can proceed.
            if let Err(e) = self.block_on(db::soft_delete_by_inode(
                &self.ctx.db,
                target_meta.inode,
            )) {
                tracing::error!("rename: failed to soft-delete target: {e:#}");
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }
        }

        // Determine new parent's file_id
        let new_parent_file_id = if new_parent_inode == db::ROOT_INODE {
            None
        } else {
            match self.block_on(db::get_parent_info(&self.ctx.db, new_parent_inode)) {
                Ok(Some((_acct, parent_file_id))) => Some(parent_file_id),
                _ => return Err(NTSTATUS::STATUS_OBJECT_PATH_NOT_FOUND),
            }
        };

        // Update the DB record
        if let Err(e) = self.block_on(db::rename_file(
            &self.ctx.db,
            meta.inode,
            new_file_name,
            new_parent_file_id.as_deref(),
        )) {
            tracing::error!("rename: DB error: {e:#}");
            return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
        }

        // Enqueue a rename task for the sync engine
        if let Ok(Some(details)) =
            self.block_on(db::get_file_details_by_inode(&self.ctx.db, meta.inode))
        {
            let _ = self.block_on(db::enqueue_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                "rename",
                details.local_path.as_deref(),
            ));

            // Rename the local cache file if present
            if let Some(ref old_path) = details.local_path {
                let old = PathBuf::from(old_path);
                if old.exists() {
                    if let Some(parent_dir) = old.parent() {
                        let new_path = parent_dir.join(new_file_name);
                        let _ = fs::rename(&old, &new_path);
                    }
                }
            }
        }

        Ok(())
    }

    fn set_file_size(
        &self,
        file_context: &Self::FileContext,
        new_size: u64,
        set_allocation_size: bool,
    ) -> WinResult<()> {
        if file_context.ino == db::ROOT_INODE {
            return Err(NTSTATUS::STATUS_INVALID_DEVICE_REQUEST);
        }

        if set_allocation_size {
            // Allocation size changes don't affect the actual file content
            return Ok(());
        }

        // Truncate or extend the local file
        let file = match fs::OpenOptions::new()
            .write(true)
            .open(&file_context.local_path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("set_file_size: failed to open: {e:#}");
                return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
            }
        };

        if let Err(e) = file.set_len(new_size) {
            tracing::error!("set_file_size: truncate failed: {e:#}");
            return Err(NTSTATUS::STATUS_INTERNAL_ERROR);
        }

        // Update DB
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let _ = self.block_on(db::update_file_size_mtime(
            &self.ctx.db,
            file_context.ino,
            new_size as i64,
            now_ms,
        ));

        Ok(())
    }

    fn can_delete(&self, file_context: &Self::FileContext, _file_name: &OsStr) -> WinResult<()> {
        if file_context.ino == db::ROOT_INODE {
            return Err(NTSTATUS::STATUS_CANNOT_DELETE);
        }

        let meta = match self.block_on(db::get_file_by_inode(&self.ctx.db, file_context.ino)) {
            Ok(Some(m)) => m,
            Ok(None) => return Err(NTSTATUS::STATUS_OBJECT_NAME_NOT_FOUND),
            Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
        };

        // Directories must be empty to delete
        if meta.mime_type == "application/vnd.google-apps.folder" {
            match self.block_on(db::has_children(&self.ctx.db, file_context.ino)) {
                Ok(true) => return Err(NTSTATUS::STATUS_DIRECTORY_NOT_EMPTY),
                Ok(false) => {}
                Err(_) => return Err(NTSTATUS::STATUS_INTERNAL_ERROR),
            }
        }

        Ok(())
    }
}

// ─── VFS Backend ────────────────────────────────────────────────────────────

/// Windows WinFSP backend that mounts a virtual drive letter.
pub struct WindowsVfsBackend;

#[async_trait]
impl VfsBackend for WindowsVfsBackend {
    async fn mount(&self, ctx: VfsContext) -> anyhow::Result<VfsHandle> {
        mount_winfsp(ctx).await
    }

    async fn unmount(handle: VfsHandle) -> anyhow::Result<()> {
        drop(handle);
        Ok(())
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Mount a WinFSP virtual filesystem at the configured mount point.
///
/// The mount point should be a drive letter path like `G:\`. If the path
/// doesn't end with `:\`, it will be treated as a directory mount point
/// (WinFSP supports both).
pub async fn mount_winfsp(ctx: VfsContext) -> anyhow::Result<VfsHandle> {
    let mount_point = ctx.mount_point.clone();
    let mount_str = mount_point.to_string_lossy().to_string();

    info!("mounting WinFSP filesystem at {}", mount_str);

    let fs = WinFspVfs::new(ctx);

    // Configure volume parameters
    let mut params = FSP_FSCTL_VOLUME_PARAMS::default();
    params.sector_size = SECTOR_SIZE;
    params.sectors_per_allocation_unit = SECTORS_PER_ALLOCATION_UNIT;
    params.volume_label = VOLUME_LABEL.to_string();

    // Create the filesystem host and mount
    let host = tokio::task::spawn_blocking(move || FileSystemHost::new(fs, &mount_str, &params))
        .await
        .map_err(|e| anyhow::anyhow!("WinFSP spawn task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("failed to mount WinFSP at {}: {e}", mount_str))?;

    info!("WinFSP filesystem mounted at {}", mount_str);

    Ok(VfsHandle::new_windows(host, mount_point))
}

/// Unmount the WinFSP filesystem at the given mount point.
pub fn unmount_winfsp(mount_point: &std::path::Path) -> anyhow::Result<()> {
    info!("unmounting WinFSP filesystem at {}", mount_point.display());
    // Dropping the FileSystemHost handle will unmount the filesystem.
    // This function exists for explicit unmount scenarios.
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_mime_detection() {
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.document"
        ));
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.spreadsheet"
        ));
        assert!(!is_google_workspace_mime("application/pdf"));
        assert!(!is_google_workspace_mime("text/plain"));
    }

    #[test]
    fn mime_guess_from_name() {
        assert_eq!(guess_mime_from_name("read.txt"), "text/plain");
        assert_eq!(guess_mime_from_name("photo.jpg"), "image/jpeg");
        assert_eq!(guess_mime_from_name("data.json"), "application/json");
        assert_eq!(guess_mime_from_name("unknown"), "application/octet-stream");
    }
}
