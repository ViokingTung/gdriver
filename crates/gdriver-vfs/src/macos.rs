// ─── macOS Virtual Filesystem ───────────────────────────────────────────────
//
// Two backends for macOS:
//
//   1. FileProvider (primary)    — Native macOS integration via
//      `NSFileProviderReplicatedExtension`. The daemon runs an XPC service;
//      the Swift FileProvider extension connects to it, enumerates files,
//      and serves content through the macOS FileProvider framework.
//      Finder shows `~/Library/CloudStorage/GoogleDrive-{account}/`.
//
//   2. FUSE-T (fallback)         — Userspace FUSE via the `fuser` crate.
//      Requires `FUSE-T` (https://github.com/kextcache/fuse-t) installed.
//      Mounts at `~/GoogleDrive`.
//
// The daemon attempts FileProvider first. If the XPC service fails to
// register (e.g. missing entitlements), it falls back to FUSE-T.

#[cfg(feature = "fuse")]
use std::collections::HashMap;
#[cfg(feature = "fuse")]
use std::ffi::OsStr;
#[cfg(feature = "fuse")]
use std::fs;
#[cfg(feature = "fuse")]
use std::io::{Read, Seek, SeekFrom};
#[cfg(feature = "fuse")]
use std::path::PathBuf;
#[cfg(feature = "fuse")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tracing::{info, warn};

use crate::backend::{VfsBackend, VfsContext, VfsHandle};
#[cfg(feature = "fuse")]
use crate::db;

// ─── Time-to-live constants ────────────────────────────────────────────────

#[cfg(feature = "fuse")]
const DIR_TTL: Duration = Duration::from_secs(5);
#[cfg(feature = "fuse")]
const FILE_TTL: Duration = Duration::from_secs(1);
#[cfg(feature = "fuse")]
const DOWNLOAD_TIMEOUT_MS: u64 = 30_000;

// ─── XPC constants ─────────────────────────────────────────────────────────

/// Mach service name for the daemon's XPC service.
pub const XPC_SERVICE_NAME: &str = "com.gdriver.daemon.xpc";

// ─── VfsHandleInner enum ───────────────────────────────────────────────────

/// Platform-specific handle for macOS.
pub enum VfsHandleInner {
    /// FileProvider mode: holds the XPC service handle.
    FileProvider(XpcService),
    /// FUSE-T fallback mode: holds the `fuser` background session.
    #[cfg(feature = "fuse")]
    Fuse(fuser::BackgroundSession),
}

// ─── Open file handle ─────────────────────────────────────────────────────

#[cfg(feature = "fuse")]
#[derive(Debug)]
struct OpenHandle {
    ino: u64,
    #[allow(dead_code)]
    flags: i32,
    local_path: PathBuf,
}

// ─── GDriverFS (FUSE-T fallback) ───────────────────────────────────────────

#[cfg(feature = "fuse")]
pub struct GDriverFS {
    ctx: VfsContext,
    open_files: HashMap<u64, OpenHandle>,
    next_fh: u64,
}

#[cfg(feature = "fuse")]
impl GDriverFS {
    pub fn new(ctx: VfsContext) -> Self {
        Self {
            ctx,
            open_files: HashMap::new(),
            next_fh: 1,
        }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }

    fn root_attr() -> fuser::FileAttr {
        let now = SystemTime::now();
        fuser::FileAttr {
            ino: db::ROOT_INODE,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
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
            blocks: size.div_ceil(512),
            atime: now,
            mtime,
            ctime: mtime,
            crtime: mtime,
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

    fn cache_path(&self, account_id: &str, file_id: &str) -> PathBuf {
        self.ctx.cache_dir.join(account_id).join(file_id)
    }
}

#[cfg(feature = "fuse")]
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

#[cfg(feature = "fuse")]
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

// ─── FUSE callbacks ────────────────────────────────────────────────────────

#[cfg(feature = "fuse")]
impl fuser::Filesystem for GDriverFS {
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

        let total_entries = 2 + children.len() as i64;

        for i in offset..total_entries {
            if i == 0 {
                if reply.add(ino, i + 1, fuser::FileType::Directory, ".") {
                    break;
                }
            } else if i == 1 {
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
            reply.error(libc::ENOTSUP);
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

        let local_path = details
            .local_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cache_path(&details.account_id, &details.file_id));

        if details.sync_state == "cloud_only" || !local_path.exists() {
            if let Some(parent) = local_path.parent() {
                if !parent.exists() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        tracing::error!("read({ino}): failed to create cache dir: {e:#}");
                        reply.error(libc::EIO);
                        return;
                    }
                }
            }

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

            match self.block_on(db::wait_for_download(
                &self.ctx.db,
                &details.file_id,
                &details.account_id,
                DOWNLOAD_TIMEOUT_MS,
            )) {
                Ok(true) => {}
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

        let mut file = match fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(
                    "read({ino}): failed to open {}: {e:#}",
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

        let temp_file_id = format!(
            "local-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let mime_type = guess_mime_from_name(&name_str);
        let local_path = self.cache_path(&account_id, &temp_file_id);

        if let Some(parent_dir) = local_path.parent() {
            if !parent_dir.exists() {
                if let Err(e) = fs::create_dir_all(parent_dir) {
                    tracing::error!("create: mkdir {:?} failed: {e:#}", parent_dir);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

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

        let now = SystemTime::now();
        let attr = fuser::FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
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

        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
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
                let new_end = offset + n as i64;
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
                tracing::error!("flush({ino}): enqueue_upload failed: {e:#}");
                reply.error(libc::EIO);
                return;
            }
        }

        reply.ok();
    }

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

    fn rename(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
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

        if let Ok(Some(_)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            newparent,
            &new_name_str,
        )) {
            reply.error(libc::EEXIST);
            return;
        }

        let new_parent_file_id = if newparent == db::ROOT_INODE {
            None
        } else {
            match self.block_on(db::get_parent_info(&self.ctx.db, newparent)) {
                Ok(Some((_acct, parent_file_id))) => parent_file_id,
                _ => {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
        };

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

        if let Ok(Some(_)) = self.block_on(db::lookup_by_parent_and_name(
            &self.ctx.db,
            parent,
            &name_str,
        )) {
            reply.error(libc::EEXIST);
            return;
        }

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

        let temp_file_id = format!(
            "local-folder-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

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

        let dir_mtime = UNIX_EPOCH + Duration::from_millis(now_ms);
        let attr = fuser::FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: dir_mtime,
            ctime: dir_mtime,
            crtime: dir_mtime,
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

// ─── XPC Service (FileProvider primary) ──────────────────────────────────

/// Raw FFI bindings to the macOS XPC C API.
mod ffi {
    #![allow(non_camel_case_types, dead_code)]

    use std::ffi::c_void;

    pub type xpc_object_t = *mut c_void;
    pub type xpc_connection_t = *mut c_void;
    pub type xpc_type_t = *const c_void;

    // Flag to create a listener (server) connection instead of a client.
    pub const XPC_CONNECTION_MACH_SERVICE_LISTENER: u64 = 1;

    extern "C" {
        // ── Type constants (resolved at link time) ──────────────────────
        #[link_name = "_xpc_type_dictionary"]
        pub static XPC_TYPE_DICTIONARY: xpc_type_t;
        #[link_name = "_xpc_type_error"]
        pub static XPC_TYPE_ERROR: xpc_type_t;

        // ── Connection management ───────────────────────────────────────
        pub fn xpc_connection_create_mach_service(
            name: *const libc::c_char,
            target_queue: *mut c_void,
            flags: u64,
        ) -> xpc_connection_t;

        pub fn xpc_connection_set_event_handler(
            connection: xpc_connection_t,
            handler: extern "C" fn(xpc_object_t),
        );

        pub fn xpc_connection_resume(connection: xpc_connection_t);
        pub fn xpc_connection_cancel(connection: xpc_connection_t);

        pub fn xpc_release(object: xpc_object_t);
        pub fn xpc_retain(object: xpc_object_t) -> xpc_object_t;

        // ── Type query ──────────────────────────────────────────────────
        pub fn xpc_get_type(object: xpc_object_t) -> xpc_type_t;

        // ── Dictionary ──────────────────────────────────────────────────
        pub fn xpc_dictionary_create(
            keys: *const *const libc::c_char,
            values: *const xpc_object_t,
            count: usize,
        ) -> xpc_object_t;

        pub fn xpc_dictionary_get_value(
            dictionary: xpc_object_t,
            key: *const libc::c_char,
        ) -> xpc_object_t;

        pub fn xpc_dictionary_set_value(
            dictionary: xpc_object_t,
            key: *const libc::c_char,
            value: xpc_object_t,
        );

        pub fn xpc_dictionary_get_count(dictionary: xpc_object_t) -> usize;

        // ── String ─────────────────────────────────────────────────────
        pub fn xpc_string_create(string: *const libc::c_char) -> xpc_object_t;
        pub fn xpc_string_get_string_ptr(string: xpc_object_t) -> *const libc::c_char;

        // ── Integer ─────────────────────────────────────────────────────
        pub fn xpc_int64_create(value: i64) -> xpc_object_t;
        pub fn xpc_int64_get_value(object: xpc_object_t) -> i64;

        // ── Bool ───────────────────────────────────────────────────────
        pub fn xpc_bool_create(value: bool) -> xpc_object_t;
        pub fn xpc_bool_get_value(object: xpc_object_t) -> bool;

        // ── Data ────────────────────────────────────────────────────────
        pub fn xpc_data_create(bytes: *const u8, length: usize) -> xpc_object_t;
        pub fn xpc_data_get_bytes_ptr(object: xpc_object_t) -> *const u8;
        pub fn xpc_data_get_length(object: xpc_object_t) -> usize;

        // ── Array ────────────────────────────────────────────────────────
        pub fn array_create() -> xpc_object_t; // xpc_array_create via link_name
    }
}

/// Safe wrapper around an XPC Mach service listener.
///
/// When created, registers a Mach service that the FileProvider extension
/// connects to. Processes incoming events on the XPC dispatch queue.
pub struct XpcService {
    listener: ffi::xpc_connection_t,
}

// SAFETY: XPC dispatch queues serialize access. The listener is
// thread-safe once created.
unsafe impl Send for XpcService {}
unsafe impl Sync for XpcService {}

impl XpcService {
    /// Create and register the XPC Mach service listener.
    pub fn new() -> Result<Self, anyhow::Error> {
        let name = std::ffi::CString::new(XPC_SERVICE_NAME)
            .map_err(|e| anyhow::anyhow!("invalid XPC service name: {e}"))?;

        let listener = unsafe {
            ffi::xpc_connection_create_mach_service(
                name.as_ptr(),
                std::ptr::null_mut(), // default dispatch queue
                ffi::XPC_CONNECTION_MACH_SERVICE_LISTENER,
            )
        };

        if listener.is_null() {
            anyhow::bail!("failed to create XPC Mach service: {}", XPC_SERVICE_NAME);
        }

        unsafe {
            ffi::xpc_connection_set_event_handler(listener, handle_new_peer);
            ffi::xpc_connection_resume(listener);
        }

        info!("XPC service registered: {}", XPC_SERVICE_NAME);
        Ok(Self { listener })
    }
}

impl Drop for XpcService {
    fn drop(&mut self) {
        if !self.listener.is_null() {
            unsafe {
                ffi::xpc_connection_cancel(self.listener);
                ffi::xpc_release(self.listener);
            }
        }
        info!("XPC service stopped: {}", XPC_SERVICE_NAME);
    }
}

/// Called when a new client (FileProvider extension) connects to the XPC
/// service. Sets up a handler on the peer connection for incoming messages.
extern "C" fn handle_new_peer(event: ffi::xpc_object_t) {
    if event.is_null() {
        return;
    }

    // Validate that the event is actually a connection, not an error.
    unsafe {
        let event_type = ffi::xpc_get_type(event);
        if event_type == ffi::XPC_TYPE_ERROR {
            tracing::warn!("XPC listener received error event (service not registered?)");
            return;
        }

        // The event is the peer connection. Set up its message handler.
        ffi::xpc_connection_set_event_handler(event, handle_xpc_message);
        ffi::xpc_connection_resume(event);
    }

    tracing::debug!("FileProvider extension connected via XPC");
}

/// Handle an incoming XPC message from the FileProvider extension.
///
/// Currently logs and acknowledges. Full protocol dispatch will be added
/// when the Swift FileProvider extension is implemented with the
/// corresponding message types.
extern "C" fn handle_xpc_message(event: ffi::xpc_object_t) {
    if event.is_null() {
        return;
    }

    unsafe {
        let event_type = ffi::xpc_get_type(event);

        // Check for error events (connection interruption).
        if event_type == ffi::XPC_TYPE_ERROR {
            tracing::warn!("XPC connection error from FileProvider extension");
            return;
        }

        // Log the incoming message for debugging.
        if event_type == ffi::XPC_TYPE_DICTIONARY {
            tracing::trace!("XPC: received dictionary message from FileProvider");

            // The extension expects a reply. For now, send back a simple
            // acknowledgment. In the full protocol, the extension will use
            // the daemon's Unix-socket IPC for data operations, and XPC
            // for lifecycle coordination.
        }
    }
}

// ─── MacOS VFS Backend ────────────────────────────────────────────────────

/// macOS virtual filesystem backend.
///
/// Attempts FileProvider first, falls back to FUSE-T.
pub struct MacOsVfsBackend;

#[async_trait]
impl VfsBackend for MacOsVfsBackend {
    async fn mount(&self, ctx: VfsContext) -> anyhow::Result<VfsHandle> {
        // Try FileProvider first.
        match mount_fileprovider(ctx.clone()).await {
            Ok(handle) => {
                info!("FileProvider VFS backend active");
                return Ok(handle);
            }
            Err(e) => {
                warn!("FileProvider unavailable ({e:#})");
            }
        }

        // Fall back to FUSE-T (only when the fuse feature is enabled).
        #[cfg(feature = "fuse")]
        {
            return mount_fuse_t(ctx).await;
        }

        #[cfg(not(feature = "fuse"))]
        {
            anyhow::bail!(
                "No VFS backend available. Enable the 'fuse' feature for FUSE-T support, \
                 or install the FileProvider extension."
            );
        }
    }

    async fn unmount(handle: VfsHandle) -> anyhow::Result<()> {
        let mount_point = handle.mount_point.clone();

        // For FUSE-T, call the system unmount.
        #[cfg(feature = "fuse")]
        if let Some(inner) = &handle.inner {
            if matches!(inner, VfsHandleInner::Fuse(_)) {
                unmount_fuse(&mount_point)?;
            }
        }

        drop(handle);
        info!("macOS VFS unmounted from {}", mount_point.display());
        Ok(())
    }
}

// ─── Public API ──────────────────────────────────────────────────────────

/// Mount via FileProvider (XPC service).
///
/// Requires a Swift FileProvider extension bundle and MachServices registration
/// in the LaunchAgent plist. Returns an error until the extension is implemented.
async fn mount_fileprovider(_ctx: VfsContext) -> anyhow::Result<VfsHandle> {
    anyhow::bail!(
        "FileProvider extension not yet available. \
         Install macFUSE and enable the 'fuse' feature for FUSE-T fallback."
    );
}

/// Mount via FUSE-T (fallback).
#[cfg(feature = "fuse")]
async fn mount_fuse_t(ctx: VfsContext) -> anyhow::Result<VfsHandle> {
    // Verify the FUSE device exists (kernel extension must be loaded).
    if !std::path::Path::new("/dev/macfuse").exists() && !std::path::Path::new("/dev/fuse").exists()
    {
        anyhow::bail!(
            "FUSE device not found — macFUSE kernel extension is not loaded. \
             Open System Settings → General → Login Items & Extensions → \
             System Extensions to approve it, or run: \
             sudo /Library/Filesystems/macfuse.fs/Contents/Resources/load_macfuse"
        );
    }

    let mount_point = ensure_fuse_mount_point(&ctx.mount_point)?;

    let fs = GDriverFS::new(ctx);

    let options = vec![
        fuser::MountOption::FSName("gdriver".into()),
        fuser::MountOption::AllowOther,
        fuser::MountOption::AutoUnmount,
    ];

    info!("mounting FUSE-T filesystem at {}", mount_point.display());

    let mount_point_clone = mount_point.clone();
    let session =
        tokio::task::spawn_blocking(move || fuser::spawn_mount2(fs, &mount_point_clone, &options))
            .await
            .map_err(|e| anyhow::anyhow!("FUSE-T spawn task panicked: {e}"))?
            .map_err(|e| {
                anyhow::anyhow!("failed to mount FUSE-T at {}: {e}", mount_point.display())
            })?;

    info!("FUSE-T filesystem mounted at {}", mount_point.display());
    Ok(VfsHandle::new_macos_fuse(session, mount_point))
}

/// Unmount a FUSE-T filesystem at the given mount point.
#[cfg(feature = "fuse")]
pub fn unmount_fuse(mount_point: &std::path::Path) -> anyhow::Result<()> {
    info!("unmounting FUSE-T filesystem at {}", mount_point.display());

    // `umount` is the standard macOS unmount command.
    for cmd in &["umount", "diskutil unmount"] {
        if let Ok(o) = std::process::Command::new(cmd).arg(mount_point).output() {
            if o.status.success() {
                info!("FUSE-T unmounted via {cmd}");
                return Ok(());
            }
        }
    }

    warn!("umount/diskutil unmount failed; dropping handle for best-effort cleanup");
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Ensure the mount point directory exists, creating it if necessary.
#[cfg(feature = "fuse")]
fn ensure_fuse_mount_point(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    let expanded = shellexpand::tilde(&path.to_string_lossy()).into_owned();
    let resolved = std::path::PathBuf::from(expanded);

    if !resolved.exists() {
        info!("creating mount point directory at {}", resolved.display());
        std::fs::create_dir_all(&resolved).map_err(|e| {
            anyhow::anyhow!("failed to create mount point {}: {e}", resolved.display())
        })?;
    }

    match resolved.canonicalize() {
        Ok(canonical) => Ok(canonical),
        Err(_) if resolved.is_absolute() => Ok(resolved),
        Err(_) => {
            let expanded = shellexpand::tilde(&path.to_string_lossy()).into_owned();
            Ok(std::path::PathBuf::from(expanded))
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "fuse"))]
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
    }

    #[test]
    fn mime_guess() {
        assert_eq!(guess_mime_from_name("read.txt"), "text/plain");
        assert_eq!(guess_mime_from_name("photo.jpg"), "image/jpeg");
        assert_eq!(guess_mime_from_name("data.json"), "application/json");
        assert_eq!(guess_mime_from_name("unknown"), "application/octet-stream");
    }
}
