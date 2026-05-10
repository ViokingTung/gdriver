use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use fuser::MountOption;
use tracing::{info, warn};

use crate::{
    backend::{VfsBackend, VfsContext, VfsHandle},
    db,
};

// ─── Time-to-live constants ────────────────────────────────────────────────

/// TTL for directory attributes (5 s). Directories change less frequently.
const DIR_TTL: Duration = Duration::from_secs(5);
/// TTL for regular file attributes (1 s). Files are more volatile.
const FILE_TTL: Duration = Duration::from_secs(1);

/// Maximum time to wait for an on-demand download before returning an error.
const DOWNLOAD_TIMEOUT_MS: u64 = 30_000;

// ─── Open file handle ──────────────────────────────────────────────────────────

/// Tracks an open file for the lifetime between `open` and `release`.
#[derive(Debug)]
struct OpenHandle {
    ino: u64,
    flags: i32,
    /// Cached local file path so `write` can avoid a DB lookup on every call.
    local_path: PathBuf,
}

// ─── GDriverFS ────────────────────────────────────────────────────────────────

/// FUSE filesystem implementation backed by Google Drive.
pub struct GDriverFS {
    /// VFS context (cache directory, mount point, DB pool).
    ctx: VfsContext,
    /// Map from FUSE file handle (fh) to open file metadata.
    open_files: HashMap<u64, OpenHandle>,
    /// Monotonically increasing file handle counter.
    next_fh: u64,
}

impl GDriverFS {
    /// Create a new FUSE filesystem instance.
    pub fn new(ctx: VfsContext) -> Self {
        Self {
            ctx,
            open_files: HashMap::new(),
            next_fh: 1,
        }
    }

    /// Execute an async future synchronously using the current Tokio runtime.
    ///
    /// FUSE callbacks are synchronous; this bridges to the async `sqlx` queries.
    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }

    /// Build `FileAttr` for the synthetic root directory (inode 1).
    fn root_attr() -> fuser::FileAttr {
        let now = SystemTime::now();
        fuser::FileAttr {
            ino: db::ROOT_INODE,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            kind: fuser::FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        }
    }

    /// Convert a [`db::FileMeta`] into an [`fuser::FileAttr`].
    fn file_to_attr(meta: &db::FileMeta) -> fuser::FileAttr {
        let mtime = UNIX_EPOCH + Duration::from_millis(meta.modified_time as u64);
        let now = SystemTime::now();

        let (kind, perm, size, nlink) = if meta.mime_type == "application/vnd.google-apps.folder" {
            (fuser::FileType::Directory, 0o755, 0, 2)
        } else {
            (fuser::FileType::RegularFile, 0o644, meta.size as u64, 1)
        };

        fuser::FileAttr {
            ino: meta.inode,
            size,
            blocks: (size + 511) / 512,
            atime: now,
            mtime,
            ctime: mtime,
            kind,
            perm,
            nlink,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        }
    }
    /// Build the local cache path for a file.
    ///
    /// Format: `{cache_dir}/{account_id}/{file_id}`
    fn cache_path(&self, account_id: &str, file_id: &str) -> PathBuf {
        self.ctx.cache_dir.join(account_id).join(file_id)
    }
}

/// Return `true` if the MIME type is a native Google Workspace format.
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

impl fuser::Filesystem for GDriverFS {
    // ── lookup ─────────────────────────────────────────────────────────────

    fn lookup(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let result = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        ));

        match result {
            Ok(Some(meta)) => {
                let attr = Self::file_to_attr(&meta);
                let ttl = if attr.kind == fuser::FileType::Directory {
                    DIR_TTL
                } else {
                    FILE_TTL
                };
                reply.entry(&ttl, &attr, 0);
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(e) => {
                tracing::error!("lookup({parent}, {name_str:?}) failed: {e:#}");
                reply.error(libc::EIO);
            }
        }
    }

    // ── getattr ────────────────────────────────────────────────────────────

    fn getattr(&mut self, _req: &fuser::Request<'_>, ino: u64, reply: fuser::ReplyAttr) {
        if ino == db::ROOT_INODE {
            reply.attr(&DIR_TTL, &Self::root_attr());
            return;
        }

        let result = self.block_on(db::get_file_by_inode(&self.ctx.db, ino));

        match result {
            Ok(Some(meta)) => {
                let attr = Self::file_to_attr(&meta);
                let ttl = if attr.kind == fuser::FileType::Directory {
                    DIR_TTL
                } else {
                    FILE_TTL
                };
                reply.attr(&ttl, &attr);
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(e) => {
                tracing::error!("getattr({ino}) failed: {e:#}");
                reply.error(libc::EIO);
            }
        }
    }

    // ── readdir ────────────────────────────────────────────────────────────

    fn readdir(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: fuser::ReplyDirectory,
    ) {
        let children = match self.block_on(db::list_children_by_inode(&self.ctx.db, ino)) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("readdir({ino}) failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        // Build the full entry list: . (self), .. (parent), then children.
        // Offset 0 → ".", 1 → "..", 2+ → children[offset-2].
        let total_entries = 2 + children.len() as i64;

        for i in offset..total_entries {
            if i == 0 {
                // "." — current directory
                if reply.add(ino, i + 1, fuser::FileType::Directory, ".") {
                    break;
                }
            } else if i == 1 {
                // ".." — parent directory (use parent=1 as a fallback).
                if reply.add(db::ROOT_INODE, i + 1, fuser::FileType::Directory, "..") {
                    break;
                }
            } else {
                let child = &children[(i - 2) as usize];
                let kind = if child.mime_type == "application/vnd.google-apps.folder" {
                    fuser::FileType::Directory
                } else {
                    fuser::FileType::RegularFile
                };

                if reply.add(child.inode, i + 1, kind, &child.name) {
                    break;
                }
            }
        }

        reply.ok();
    }

    // ── open ────────────────────────────────────────────────────────────────

    fn open(&mut self, _req: &fuser::Request<'_>, ino: u64, flags: i32, reply: fuser::ReplyOpen) {
        if ino == db::ROOT_INODE {
            reply.error(libc::EISDIR);
            return;
        }

        let details = match self.block_on(db::get_file_details_by_inode(&self.ctx.db, ino)) {
            Ok(Some(d)) => d,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                tracing::error!("open({ino}) DB error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        if details.mime_type == "application/vnd.google-apps.folder" {
            reply.error(libc::EISDIR);
            return;
        }

        if is_google_workspace_mime(&details.mime_type) {
            reply.error(libc::EOPNOTSUPP);
            return;
        }

        let local_path = details
            .local_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cache_path(&details.account_id, &details.file_id));

        let fh = self.next_fh;
        self.next_fh += 1;
        self.open_files.insert(
            fh,
            OpenHandle {
                ino,
                flags,
                local_path,
            },
        );

        reply.opened(fh, 0);
    }

    // ── read ────────────────────────────────────────────────────────────────

    fn read(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyData,
    ) {
        // Validate the handle.
        let handle = match self.open_files.get(&fh) {
            Some(h) => h,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        if handle.ino != ino {
            reply.error(libc::EBADF);
            return;
        }

        // Look up file details.
        let details = match self.block_on(db::get_file_details_by_inode(&self.ctx.db, ino)) {
            Ok(Some(d)) => d,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                tracing::error!("read({ino}) DB error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        // Determine the local file path: use existing local_path or compute
        // the cache path.
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
                        tracing::error!("read({ino}): failed to create cache dir: {e:#}");
                        reply.error(libc::EIO);
                        return;
                    }
                }
            }

            // Enqueue a download task for the sync engine to pick up.
            if let Err(e) = self.block_on(db::enqueue_download_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                &local_path.to_string_lossy(),
            )) {
                tracing::error!("read({ino}): failed to enqueue download: {e:#}");
                reply.error(libc::EIO);
                return;
            }

            // Block until the sync engine completes the download (or timeout).
            match self.block_on(db::wait_for_download(
                &self.ctx.db,
                &details.file_id,
                &details.account_id,
                DOWNLOAD_TIMEOUT_MS,
            )) {
                Ok(true) => {} // Download complete.
                Ok(false) => {
                    tracing::warn!("read({ino}): download timed out or failed");
                    reply.error(libc::EIO);
                    return;
                }
                Err(e) => {
                    tracing::error!("read({ino}): download wait error: {e:#}");
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        // Read from the local file.
        let mut file = match fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "read({ino}): failed to open local file {}: {e:#}",
                    local_path.display()
                );
                reply.error(libc::EIO);
                return;
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset as u64)) {
            tracing::error!("read({ino}): seek to {offset} failed: {e:#}");
            reply.error(libc::EIO);
            return;
        }

        let mut buf = vec![0u8; size as usize];
        match file.read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                reply.data(&buf);
            }
            Err(e) => {
                tracing::error!("read({ino}): read failed: {e:#}");
                reply.error(libc::EIO);
            }
        }
    }

    // ── create ──────────────────────────────────────────────────────────────

    fn create(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Determine the parent's account_id and file_id.
        let (account_id, parent_file_id) =
            match self.block_on(db::get_parent_info(&self.ctx.db, parent)) {
                Ok(Some(info)) => info,
                Ok(None) if parent == db::ROOT_INODE => {
                    // Creating at My Drive root — find an account.
                    match self.block_on(db::get_first_account_id(&self.ctx.db)) {
                        Ok(Some(acct)) => (acct, None),
                        Ok(None) => {
                            reply.error(libc::EIO);
                            return;
                        }
                        Err(e) => {
                            tracing::error!("create: get_first_account_id failed: {e:#}");
                            reply.error(libc::EIO);
                            return;
                        }
                    }
                }
                Ok(None) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    tracing::error!("create: get_parent_info({parent}) failed: {e:#}");
                    reply.error(libc::EIO);
                    return;
                }
            };

        // Generate a temporary file ID for this locally-created file.
        let temp_file_id = format!(
            "local-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let mime_type = guess_mime_from_name(&name_str);
        let local_path = self.cache_path(&account_id, &temp_file_id);

        // Ensure the cache directory exists.
        if let Some(parent_dir) = local_path.parent() {
            if !parent_dir.exists() {
                if let Err(e) = fs::create_dir_all(parent_dir) {
                    tracing::error!("create: mkdir {:?} failed: {e:#}", parent_dir);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        // Insert a row into drive_files.
        let ino = match self.block_on(db::insert_local_file(
            &self.ctx.db,
            &temp_file_id,
            &account_id,
            &name_str,
            &mime_type,
            parent_file_id.as_deref(),
            &local_path.to_string_lossy(),
        )) {
            Ok(ino) => ino,
            Err(e) => {
                tracing::error!("create: insert_local_file failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        // Create an empty file on disk.
        if let Err(e) = fs::File::create(&local_path) {
            tracing::error!(
                "create: failed to create file {}: {e:#}",
                local_path.display()
            );
            reply.error(libc::EIO);
            return;
        }

        let fh = self.next_fh;
        self.next_fh += 1;
        self.open_files.insert(
            fh,
            OpenHandle {
                ino,
                flags,
                local_path,
            },
        );

        let attr = fuser::FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            kind: fuser::FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        };

        reply.created(&FILE_TTL, &attr, 0, fh, 0);
    }

    // ── write ───────────────────────────────────────────────────────────────

    fn write(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        let handle = match self.open_files.get(&fh) {
            Some(h) if h.ino == ino => h,
            _ => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // Open the local file for writing at the given offset.
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&handle.local_path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("write({ino}): failed to open local file: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset as u64)) {
            tracing::error!("write({ino}): seek to {offset} failed: {e:#}");
            reply.error(libc::EIO);
            return;
        }

        match std::io::Write::write(&mut file, data) {
            Ok(n) => {
                // Update the cached size and mtime in the DB.
                let new_end = offset as i64 + n as i64;
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let _ = self.block_on(db::update_file_size_mtime(
                    &self.ctx.db,
                    ino,
                    new_end,
                    now_ms,
                ));

                reply.written(n as u32);
            }
            Err(e) => {
                tracing::error!("write({ino}): write failed: {e:#}");
                reply.error(libc::EIO);
            }
        }
    }

    // ── flush ───────────────────────────────────────────────────────────────

    fn flush(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: u64,
        fh: u64,
        _lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
        let handle = match self.open_files.get(&fh) {
            Some(h) if h.ino == ino => h,
            _ => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // Enqueue an upload task so the sync engine pushes the file to Drive.
        let details = match self.block_on(db::get_file_details_by_inode(&self.ctx.db, ino)) {
            Ok(Some(d)) => d,
            _ => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // Only enqueue an upload for locally-modified files.
        if details.sync_state == "modified" {
            if let Err(e) = self.block_on(db::enqueue_upload_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                &handle.local_path.to_string_lossy(),
            )) {
                tracing::error!("flush({ino}): enqueue_upload failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        }

        reply.ok();
    }

    // ── fsync ───────────────────────────────────────────────────────────────

    fn fsync(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: u64,
        fh: u64,
        _datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        let handle = match self.open_files.get(&fh) {
            Some(h) if h.ino == ino => h,
            _ => {
                reply.error(libc::EBADF);
                return;
            }
        };

        let details = match self.block_on(db::get_file_details_by_inode(&self.ctx.db, ino)) {
            Ok(Some(d)) => d,
            _ => {
                reply.error(libc::EBADF);
                return;
            }
        };

        if details.sync_state == "modified" {
            if let Err(e) = self.block_on(db::enqueue_upload_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                &handle.local_path.to_string_lossy(),
            )) {
                tracing::error!("fsync({ino}): enqueue_upload failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        }

        reply.ok();
    }

    // ── unlink ──────────────────────────────────────────────────────────────

    fn unlink(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let meta = match self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        )) {
            Ok(Some(m)) => m,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                tracing::error!("unlink({parent}, {name_str:?}) DB error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        if meta.mime_type == "application/vnd.google-apps.folder" {
            reply.error(libc::EISDIR);
            return;
        }

        // Look up full details for the account_id and local_path.
        if let Ok(Some(details)) =
            self.block_on(db::get_file_details_by_inode(&self.ctx.db, meta.inode))
        {
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

        if let Err(e) = self.block_on(db::soft_delete_by_inode(&self.ctx.db, meta.inode)) {
            tracing::error!("unlink({parent}, {name_str:?}) soft_delete failed: {e:#}");
            reply.error(libc::EIO);
            return;
        }

        reply.ok();
    }

    // ── rename ──────────────────────────────────────────────────────────────

    fn rename(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        let new_name_str = match newname.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Look up the source file.
        let meta = match self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        )) {
            Ok(Some(m)) => m,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                tracing::error!("rename({parent}, {name_str:?}) DB error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        // Check that the target name doesn't already exist under newparent.
        if let Ok(Some(_)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            newparent,
            &new_name_str,
        )) {
            reply.error(libc::EEXIST);
            return;
        }

        // Determine the new parent's file_id.
        let new_parent_file_id = if newparent == db::ROOT_INODE {
            None
        } else {
            match self.block_on(db::get_parent_info(&self.ctx.db, newparent)) {
                Ok(Some((_acct, parent_file_id))) => Some(parent_file_id),
                _ => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

        // Update the DB record.
        if let Err(e) = self.block_on(db::rename_file(
            &self.ctx.db,
            meta.inode,
            &new_name_str,
            new_parent_file_id.as_deref(),
        )) {
            tracing::error!("rename({parent}, {name_str:?}) DB error: {e:#}");
            reply.error(libc::EIO);
            return;
        }

        // Enqueue a rename task.
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

            // Rename the local cache file if present.
            if let Some(ref old_path) = details.local_path {
                let old = std::path::PathBuf::from(old_path);
                if old.exists() {
                    if let Some(parent_dir) = old.parent() {
                        let new_path = parent_dir.join(&new_name_str);
                        let _ = fs::rename(&old, &new_path);
                    }
                }
            }
        }

        reply.ok();
    }

    // ── mkdir ───────────────────────────────────────────────────────────────

    fn mkdir(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: fuser::ReplyEntry,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Check that a file with this name doesn't already exist.
        if let Ok(Some(_)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        )) {
            reply.error(libc::EEXIST);
            return;
        }

        // Determine the parent's account_id and file_id.
        let (account_id, parent_file_id) =
            match self.block_on(db::get_parent_info(&self.ctx.db, parent)) {
                Ok(Some(info)) => info,
                Ok(None) if parent == db::ROOT_INODE => {
                    match self.block_on(db::get_first_account_id(&self.ctx.db)) {
                        Ok(Some(acct)) => (acct, None),
                        Ok(None) => {
                            reply.error(libc::EIO);
                            return;
                        }
                        Err(e) => {
                            tracing::error!("mkdir: get_first_account_id failed: {e:#}");
                            reply.error(libc::EIO);
                            return;
                        }
                    }
                }
                Ok(None) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    tracing::error!("mkdir: get_parent_info({parent}) failed: {e:#}");
                    reply.error(libc::EIO);
                    return;
                }
            };

        // Generate a temporary file ID.
        let temp_file_id = format!(
            "local-folder-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        // Insert a folder row (no local_path needed).
        let ino = match self.block_on(db::insert_local_folder(
            &self.ctx.db,
            &temp_file_id,
            &account_id,
            &name_str,
            parent_file_id.as_deref(),
        )) {
            Ok(ino) => ino,
            Err(e) => {
                tracing::error!("mkdir: insert failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let attr = fuser::FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: UNIX_EPOCH + Duration::from_millis(now_ms),
            ctime: UNIX_EPOCH + Duration::from_millis(now_ms),
            kind: fuser::FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        };

        reply.entry(&DIR_TTL, &attr, 0);
    }

    // ── rmdir ───────────────────────────────────────────────────────────────

    fn rmdir(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        let name_str = match name.to_str() {
            Some(s) => s.to_string(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let meta = match self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        )) {
            Ok(Some(m)) => m,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                tracing::error!("rmdir({parent}, {name_str:?}) DB error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        };

        if meta.mime_type != "application/vnd.google-apps.folder" {
            reply.error(libc::ENOTDIR);
            return;
        }

        // Check that the directory is empty.
        match self.block_on(db::has_children(&self.ctx.db, meta.inode)) {
            Ok(true) => {
                reply.error(libc::ENOTEMPTY);
                return;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::error!("rmdir({parent}, {name_str:?}) has_children error: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        }

        // Enqueue a delete task.
        if let Ok(Some(details)) =
            self.block_on(db::get_file_details_by_inode(&self.ctx.db, meta.inode))
        {
            let _ = self.block_on(db::enqueue_task(
                &self.ctx.db,
                &details.account_id,
                &details.file_id,
                "delete",
                details.local_path.as_deref(),
            ));
        }

        if let Err(e) = self.block_on(db::soft_delete_by_inode(&self.ctx.db, meta.inode)) {
            tracing::error!("rmdir({parent}, {name_str:?}) soft_delete failed: {e:#}");
            reply.error(libc::EIO);
            return;
        }

        reply.ok();
    }

    // ── release ─────────────────────────────────────────────────────────────

    fn release(
        &mut self,
        _req: &fuser::Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.open_files.remove(&fh);
        reply.ok();
    }
}

// ─── Linux VFS backend ────────────────────────────────────────────────────────

/// Linux FUSE backend that spawns a background session managed by `fuser`.
pub struct LinuxVfsBackend;

#[async_trait]
impl VfsBackend for LinuxVfsBackend {
    async fn mount(&self, ctx: VfsContext) -> anyhow::Result<VfsHandle> {
        mount_fuse(ctx).await
    }

    async fn unmount(handle: VfsHandle) -> anyhow::Result<()> {
        drop(handle);
        Ok(())
    }
}

// ─── Public API ────────────────────────────────────────────────────────────────

/// Mount a FUSE filesystem at the configured mount point.
///
/// Creates the mount point directory if it doesn't exist, then spawns a
/// background FUSE session that serves filesystem requests on a dedicated
/// thread pool.
pub async fn mount_fuse(ctx: VfsContext) -> anyhow::Result<VfsHandle> {
    let mount_point = ensure_mount_point(&ctx.mount_point)?;

    let fs = GDriverFS::new(ctx);

    let options = vec![
        MountOption::FSName("gdriver".into()),
        MountOption::AllowOther,
        MountOption::AutoUnmount,
        MountOption::DefaultPermissions,
    ];

    info!("mounting FUSE filesystem at {}", mount_point.display());

    let mount_point_clone = mount_point.clone();
    let session =
        tokio::task::spawn_blocking(move || fuser::spawn_mount2(fs, &mount_point_clone, &options))
            .await
            .map_err(|e| anyhow::anyhow!("FUSE spawn task panicked: {e}"))?
            .map_err(|e| {
                anyhow::anyhow!("failed to mount FUSE at {}: {e}", mount_point.display())
            })?;

    info!("FUSE filesystem mounted at {}", mount_point.display());

    Ok(VfsHandle::new_linux(session, mount_point))
}

/// Unmount the FUSE filesystem at the given mount point.
///
/// Uses `fusermount -u` as a fallback if the session handle is not available.
pub fn unmount_fuse(mount_point: &std::path::Path) -> anyhow::Result<()> {
    info!("unmounting FUSE filesystem at {}", mount_point.display());

    let output = std::process::Command::new("fusermount")
        .arg("-u")
        .arg(mount_point)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            info!("FUSE filesystem unmounted successfully");
            Ok(())
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!("fusermount -u failed: {stderr}");
            Err(anyhow::anyhow!("fusermount -u failed: {stderr}"))
        }
        Err(e) => {
            warn!("fusermount command not available: {e}");
            Err(anyhow::anyhow!("fusermount not found: {e}"))
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Ensure the mount point directory exists, creating it if necessary.
///
/// Returns the canonicalized path, or the original path if canonicalization
/// fails (e.g. the parent directory doesn't exist yet).
fn ensure_mount_point(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    if !path.exists() {
        info!("creating mount point directory at {}", path.display());
        std::fs::create_dir_all(path)
            .map_err(|e| anyhow::anyhow!("failed to create mount point {}: {e}", path.display()))?;
    }

    match path.canonicalize() {
        Ok(canonical) => Ok(canonical),
        Err(_) => {
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                let expanded = shellexpand::tilde(&path.to_string_lossy()).into_owned();
                Ok(PathBuf::from(expanded))
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("in-memory pool");

        // Run the init migration so the drive_files table exists.
        sqlx::migrate!("../gdriver-daemon/migrations")
            .run(&pool)
            .await
            .expect("migrations");

        pool
    }

    /// Seed an account + a few files to make DB queries meaningful.
    async fn seed_test_data(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO accounts (id, email, created_at, last_used_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind("test-account")
        .bind("test@example.com")
        .bind(1_700_000_000_000_i64)
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();

        // Root file (folder)
        sqlx::query(
            "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size, modified_time, is_trashed)
             VALUES (?, ?, ?, ?, NULL, 0, ?, 0)",
        )
        .bind("folder-1")
        .bind("test-account")
        .bind("Documents")
        .bind("application/vnd.google-apps.folder")
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();

        // Root file (regular)
        sqlx::query(
            "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size, modified_time, is_trashed)
             VALUES (?, ?, ?, ?, NULL, ?, ?, 0)",
        )
        .bind("file-1")
        .bind("test-account")
        .bind("readme.txt")
        .bind("text/plain")
        .bind(1024_i64)
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();

        // Child of folder-1
        sqlx::query(
            "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size, modified_time, is_trashed)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0)",
        )
        .bind("file-2")
        .bind("test-account")
        .bind("notes.txt")
        .bind("text/plain")
        .bind("folder-1")
        .bind(512_i64)
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();
    }

    // ── DB query tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn root_lookup_finds_toplevel_file() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        let meta = db::lookup_by_parent_and_name(&pool, db::ROOT_INODE, "readme.txt")
            .await
            .unwrap()
            .expect("should find readme.txt at root");
        assert_eq!(meta.file_id, "file-1");
        assert_eq!(meta.size, 1024);
    }

    #[tokio::test]
    async fn subdir_lookup_finds_child() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        // First find folder-1's inode
        let folder = db::lookup_by_parent_and_name(&pool, db::ROOT_INODE, "Documents")
            .await
            .unwrap()
            .expect("should find Documents");

        let meta = db::lookup_by_parent_and_name(&pool, folder.inode, "notes.txt")
            .await
            .unwrap()
            .expect("should find notes.txt under Documents");
        assert_eq!(meta.file_id, "file-2");
        assert_eq!(meta.size, 512);
    }

    #[tokio::test]
    async fn lookup_returns_none_for_unknown_name() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        let meta = db::lookup_by_parent_and_name(&pool, db::ROOT_INODE, "nonexistent.txt")
            .await
            .unwrap();
        assert!(meta.is_none());
    }

    #[tokio::test]
    async fn get_by_inode_finds_file() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        // Find readme's inode first
        let readme = db::lookup_by_parent_and_name(&pool, db::ROOT_INODE, "readme.txt")
            .await
            .unwrap()
            .unwrap();

        let meta = db::get_file_by_inode(&pool, readme.inode)
            .await
            .unwrap()
            .expect("should find by inode");
        assert_eq!(meta.name, "readme.txt");
    }

    #[tokio::test]
    async fn get_by_inode_returns_none_for_invalid() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        let meta = db::get_file_by_inode(&pool, 99999).await.unwrap();
        assert!(meta.is_none());
    }

    #[tokio::test]
    async fn list_root_children() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        let children = db::list_children_by_inode(&pool, db::ROOT_INODE)
            .await
            .unwrap();
        // Should have Documents + readme.txt
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "Documents");
        assert_eq!(children[1].name, "readme.txt");
    }

    #[tokio::test]
    async fn list_subdir_children() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        let folder = db::lookup_by_parent_and_name(&pool, db::ROOT_INODE, "Documents")
            .await
            .unwrap()
            .unwrap();

        let children = db::list_children_by_inode(&pool, folder.inode)
            .await
            .unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "notes.txt");
    }

    #[tokio::test]
    async fn trashed_files_are_excluded() {
        let pool = test_pool().await;
        seed_test_data(&pool).await;

        // Mark readme.txt as trashed
        sqlx::query("UPDATE drive_files SET is_trashed = 1 WHERE id = ?")
            .bind("file-1")
            .execute(&pool)
            .await
            .unwrap();

        let children = db::list_children_by_inode(&pool, db::ROOT_INODE)
            .await
            .unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Documents");
    }

    // ── FileAttr conversion ────────────────────────────────────────────────

    #[test]
    fn folder_attr_has_directory_kind() {
        let meta = db::FileMeta {
            inode: 5,
            file_id: "f1".into(),
            name: "MyFolder".into(),
            mime_type: "application/vnd.google-apps.folder".into(),
            size: 0,
            modified_time: 1_700_000_000_000,
        };
        let attr = GDriverFS::file_to_attr(&meta);
        assert_eq!(attr.kind, fuser::FileType::Directory);
        assert_eq!(attr.perm, 0o755);
        assert_eq!(attr.nlink, 2);
        assert_eq!(attr.size, 0);
    }

    #[test]
    fn regular_file_attr_has_file_kind() {
        let meta = db::FileMeta {
            inode: 5,
            file_id: "f1".into(),
            name: "doc.pdf".into(),
            mime_type: "application/pdf".into(),
            size: 4096,
            modified_time: 1_700_000_000_000,
        };
        let attr = GDriverFS::file_to_attr(&meta);
        assert_eq!(attr.kind, fuser::FileType::RegularFile);
        assert_eq!(attr.perm, 0o644);
        assert_eq!(attr.nlink, 1);
        assert_eq!(attr.size, 4096);
    }

    #[test]
    fn root_attr_is_directory() {
        let attr = GDriverFS::root_attr();
        assert_eq!(attr.ino, db::ROOT_INODE);
        assert_eq!(attr.kind, fuser::FileType::Directory);
        assert_eq!(attr.perm, 0o755);
    }

    // ── Infrastructure ─────────────────────────────────────────────────────

    #[test]
    fn ensure_mount_point_creates_directory() {
        let dir = std::env::temp_dir().join("gdriver_vfs_test_mount");
        let _ = std::fs::remove_dir_all(&dir);

        assert!(!dir.exists());
        let result = ensure_mount_point(&dir);
        assert!(result.is_ok());
        assert!(dir.exists());

        let result2 = ensure_mount_point(&dir);
        assert!(result2.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vfs_handle_drop_cleans_up() {
        let handle = VfsHandle {
            inner: None,
            mount_point: std::env::temp_dir().join("gdriver_drop_test"),
        };
        drop(handle);
    }
}
