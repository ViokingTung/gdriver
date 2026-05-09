//! Conflict detection algorithm.
//!
//! A conflict occurs when both the local file and the remote Drive file have
//! been modified since the last sync.  Resolution keeps both versions: the
//! local copy is renamed with a "conflict copy" suffix and uploaded separately,
//! while the remote version is downloaded to the original path.

use std::path::Path;

// ─── Detection ─────────────────────────────────────────────────────────────────

/// Check whether both the local file and the remote Drive file have changed
/// since the last sync.
///
/// Arguments:
/// * `current_local_mtime_ms` — the local file's mtime right now (Unix ms).
/// * `cached_local_mtime_ms` — `Some(ms)` when we have a stored local mtime from
///   the last sync; `None` means the file has never been synced locally.
/// * `cached_remote_etag` — `Some(etag)` when we have a stored etag from the
///   last sync; `None` means the metadata was incomplete (should not happen
///   but is handled gracefully).
/// * `current_remote_etag` — the etag returned by the Drive API right now.
pub fn detect_conflict(
    current_local_mtime_ms: i64,
    cached_local_mtime_ms: Option<i64>,
    cached_remote_etag: Option<&str>,
    current_remote_etag: &str,
) -> bool {
    let local_changed = cached_local_mtime_ms
        .map(|stored| current_local_mtime_ms > stored)
        .unwrap_or(false);

    let remote_changed = cached_remote_etag
        .map(|stored| stored != current_remote_etag)
        .unwrap_or(false);

    local_changed && remote_changed
}

// ─── Conflict copy name generation ────────────────────────────────────────────

/// Generate a conflict-copy filename matching the macOS Drive client convention:
/// `{stem} (conflict copy YYYY-MM-DD HH:MM:SS){ext}`.
pub fn conflict_copy_name(original_path: &Path, timestamp: &str) -> String {
    let stem = original_path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("Untitled"));

    let ext = original_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    format!("{stem} (conflict copy {timestamp}){ext}")
}

// ─── MIME type guessing ──────────────────────────────────────────────────────

/// Guess a MIME type from a file extension.  Used for uploading conflict
/// copies when the original MIME type is not available.
pub fn guess_mime(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
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
        Some("docx") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Conflict detection ─────────────────────────────────────────────────

    #[test]
    fn no_conflict_when_local_unchanged() {
        assert!(!detect_conflict(
            1_700_000_000_000,           // current local mtime
            Some(1_700_000_000_000),     // cached local mtime (same → unchanged)
            Some("\"etag-1\""),          // cached etag
            "\"etag-2\"",                // current remote etag (changed)
        ));
    }

    #[test]
    fn no_conflict_when_remote_unchanged() {
        assert!(!detect_conflict(
            1_700_000_001_000,           // local changed
            Some(1_700_000_000_000),
            Some("\"etag-1\""),          // cached etag
            "\"etag-1\"",                // remote unchanged
        ));
    }

    #[test]
    fn conflict_when_both_changed() {
        assert!(detect_conflict(
            1_700_000_001_000,           // local changed
            Some(1_700_000_000_000),
            Some("\"etag-1\""),
            "\"etag-2\"",                // remote changed
        ));
    }

    #[test]
    fn no_conflict_when_neither_changed() {
        assert!(!detect_conflict(
            1_700_000_000_000,
            Some(1_700_000_000_000),
            Some("\"etag-1\""),
            "\"etag-1\"",
        ));
    }

    #[test]
    fn no_conflict_when_no_cached_local_mtime() {
        // File was never synced locally — can't have a local change.
        assert!(!detect_conflict(
            1_700_000_001_000,
            None,
            Some("\"etag-1\""),
            "\"etag-2\"",
        ));
    }

    #[test]
    fn no_conflict_when_no_cached_etag() {
        // No cached etag — can't determine if remote changed.
        assert!(!detect_conflict(
            1_700_000_001_000,
            Some(1_700_000_000_000),
            None,
            "\"etag-2\"",
        ));
    }

    #[test]
    fn no_conflict_when_local_older_than_cached() {
        // Local mtime is older than the cached one (clock skew or restore).
        assert!(!detect_conflict(
            1_700_000_000_000,           // older than cached
            Some(1_700_000_001_000),
            Some("\"etag-1\""),
            "\"etag-2\"",
        ));
    }

    #[test]
    fn no_conflict_when_local_equal_and_remote_changed() {
        // Local timestamp matches cached → only remote changed → no conflict.
        assert!(!detect_conflict(
            1_700_000_000_000,
            Some(1_700_000_000_000),
            Some("\"etag-1\""),
            "\"etag-999\"",
        ));
    }

    #[test]
    fn conflict_with_etag_containing_special_chars() {
        // Etags can contain special characters (e.g. from Google Drive).
        assert!(detect_conflict(
            1_700_000_001_000,
            Some(1_700_000_000_000),
            Some("\"abc\\\"def\\\"ghi\""),
            "\"xyz\"",
        ));
    }

    #[test]
    fn no_conflict_when_etags_identical_with_special_chars() {
        assert!(!detect_conflict(
            1_700_000_000_000,
            Some(1_700_000_000_000),
            Some("\"abc\\\"def\""),
            "\"abc\\\"def\"",
        ));
    }

    #[test]
    fn conflict_with_zero_timestamps() {
        // Unix epoch.  Both changed from epoch → conflict.
        assert!(detect_conflict(
            1000,
            Some(0),
            Some("\"old\""),
            "\"new\"",
        ));
    }

    #[test]
    fn no_conflict_with_zero_timestamps_both_unchanged() {
        assert!(!detect_conflict(0, Some(0), Some("\"etag\""), "\"etag\""));
    }

    #[test]
    fn conflict_with_very_large_timestamps() {
        // Year 3000+ timestamps — should still work.
        let future = 40_000_000_000_000_i64;
        assert!(detect_conflict(
            future,
            Some(future - 1000),
            Some("\"a\""),
            "\"b\"",
        ));
    }

    #[test]
    fn no_conflict_with_empty_etag_strings() {
        // Empty etags are unusual but should be handled.
        assert!(!detect_conflict(
            1_700_000_001_000,
            Some(1_700_000_000_000),
            Some(""),
            "",
        ));
    }

    #[test]
    fn conflict_with_empty_and_non_empty_etags() {
        assert!(detect_conflict(
            1_700_000_001_000,
            Some(1_700_000_000_000),
            Some(""),
            "\"new-etag\"",
        ));
    }

    #[test]
    fn no_conflict_when_local_just_synced() {
        // Local mtime equals cached mtime (within same millisecond).
        assert!(!detect_conflict(
            1_700_000_000_000,
            Some(1_700_000_000_000),
            Some("\"etag-a\""),
            "\"etag-b\"",
        ));
    }

    #[test]
    fn conflict_when_local_one_ms_newer() {
        // One millisecond difference is enough for a conflict.
        assert!(detect_conflict(
            1_700_000_000_001,
            Some(1_700_000_000_000),
            Some("\"etag-a\""),
            "\"etag-b\"",
        ));
    }

    // ── Conflict copy name generation ──────────────────────────────────────

    #[test]
    fn conflict_copy_name_simple_file() {
        let path = Path::new("/tmp/report.txt");
        let name = conflict_copy_name(path, "2026-05-05 14:30:00");
        assert_eq!(name, "report (conflict copy 2026-05-05 14:30:00).txt");
    }

    #[test]
    fn conflict_copy_name_no_extension() {
        let path = Path::new("/tmp/README");
        let name = conflict_copy_name(path, "2026-05-05 14:30:00");
        assert_eq!(name, "README (conflict copy 2026-05-05 14:30:00)");
    }

    #[test]
    fn conflict_copy_name_multiple_dots() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = conflict_copy_name(path, "2026-05-05 14:30:00");
        // Only the last extension is split off.
        assert_eq!(name, "archive.tar (conflict copy 2026-05-05 14:30:00).gz");
    }

    #[test]
    fn conflict_copy_name_hidden_file() {
        let path = Path::new("/tmp/.gitignore");
        let name = conflict_copy_name(path, "2026-05-05 14:30:00");
        assert_eq!(
            name,
            ".gitignore (conflict copy 2026-05-05 14:30:00)"
        );
    }

    #[test]
    fn conflict_copy_name_root_file() {
        // File in root directory with no parent.
        let path = Path::new("Makefile");
        let name = conflict_copy_name(path, "2026-05-05 14:30:00");
        assert_eq!(name, "Makefile (conflict copy 2026-05-05 14:30:00)");
    }

    #[test]
    fn conflict_copy_name_with_special_timestamp() {
        let path = Path::new("/tmp/data.csv");
        let name = conflict_copy_name(path, "2025-12-31 23:59:59");
        assert_eq!(name, "data (conflict copy 2025-12-31 23:59:59).csv");
    }

    // ── MIME type guessing ─────────────────────────────────────────────────

    #[test]
    fn guess_mime_text_plain() {
        assert_eq!(guess_mime(Path::new("file.txt")), "text/plain");
    }

    #[test]
    fn guess_mime_html() {
        assert_eq!(guess_mime(Path::new("page.html")), "text/html");
        assert_eq!(guess_mime(Path::new("page.htm")), "text/html");
    }

    #[test]
    fn guess_mime_javascript() {
        assert_eq!(guess_mime(Path::new("app.js")), "text/javascript");
    }

    #[test]
    fn guess_mime_json() {
        assert_eq!(guess_mime(Path::new("data.json")), "application/json");
    }

    #[test]
    fn guess_mime_pdf() {
        assert_eq!(guess_mime(Path::new("doc.pdf")), "application/pdf");
    }

    #[test]
    fn guess_mime_images() {
        assert_eq!(guess_mime(Path::new("img.png")), "image/png");
        assert_eq!(guess_mime(Path::new("img.jpg")), "image/jpeg");
        assert_eq!(guess_mime(Path::new("img.jpeg")), "image/jpeg");
        assert_eq!(guess_mime(Path::new("img.gif")), "image/gif");
        assert_eq!(guess_mime(Path::new("img.svg")), "image/svg+xml");
    }

    #[test]
    fn guess_mime_audio_video() {
        assert_eq!(guess_mime(Path::new("song.mp3")), "audio/mpeg");
        assert_eq!(guess_mime(Path::new("video.mp4")), "video/mp4");
    }

    #[test]
    fn guess_mime_office_documents() {
        assert_eq!(guess_mime(Path::new("doc.doc")), "application/msword");
        assert_eq!(
            guess_mime(Path::new("doc.docx")),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(guess_mime(Path::new("sheet.xls")), "application/vnd.ms-excel");
        assert_eq!(
            guess_mime(Path::new("sheet.xlsx")),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
    }

    #[test]
    fn guess_mime_archives() {
        assert_eq!(guess_mime(Path::new("archive.zip")), "application/zip");
        assert_eq!(guess_mime(Path::new("archive.gz")), "application/gzip");
        assert_eq!(guess_mime(Path::new("archive.tar")), "application/gzip");
    }

    #[test]
    fn guess_mime_no_extension() {
        assert_eq!(
            guess_mime(Path::new("Makefile")),
            "application/octet-stream"
        );
    }

    #[test]
    fn guess_mime_unknown_extension() {
        assert_eq!(
            guess_mime(Path::new("data.xyzunknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn guess_mime_case_sensitive_extension() {
        // Extensions are matched case-sensitively (as implemented).
        // .TXT won't match .txt — this is the current behavior.
        assert_eq!(
            guess_mime(Path::new("FILE.TXT")),
            "application/octet-stream"
        );
    }
}
