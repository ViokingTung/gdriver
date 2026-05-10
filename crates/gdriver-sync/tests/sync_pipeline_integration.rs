//! Integration tests: sync pipeline — discovery → download → conflict detection.
//!
//! Tests the composition of modules from gdriver-sync working together
//! to simulate a complete file sync lifecycle.

use gdriver_sync::{
    conflict::{conflict_copy_name, detect_conflict, guess_mime},
    downloader::{
        extension_for_export_mime, is_workspace_document, temp_download_path,
        workspace_export_mime, MAX_RETRIES as DL_MAX_RETRIES,
    },
    uploader::{
        chunk_count, chunk_range, is_valid_chunk_size, recommended_chunk_size, CHUNK_SIZE,
        MAX_RETRIES as UL_MAX_RETRIES, MIN_CHUNK_SIZE, RESUMABLE_THRESHOLD,
    },
};

// ── Simulated sync lifecycle ───────────────────────────────────────────────

/// Simulates a file being discovered from the Drive API, checked for sync
/// status, and queued for download.
#[test]
fn discover_regular_file_and_prepare_download() {
    // A regular PDF file discovered from Drive
    let mime = "application/pdf";

    // Regular files can be downloaded directly
    assert!(!is_workspace_document(mime));
    assert_eq!(workspace_export_mime(mime), None);

    // Build temp path for download
    let target = std::path::Path::new("/home/user/GoogleDrive/report.pdf");
    let tmp = temp_download_path(target);
    assert_eq!(tmp, "/home/user/GoogleDrive/report.pdf.gdriver-tmp");

    // Verify file extension detection
    let ext = extension_for_export_mime(mime);
    assert_eq!(ext, ".pdf");
}

#[test]
fn discover_workspace_document_and_prepare_export() {
    // A Google Doc discovered from Drive
    let drive_mime = "application/vnd.google-apps.document";

    // Must be exported
    assert!(is_workspace_document(drive_mime));

    let export_mime = workspace_export_mime(drive_mime).unwrap();
    assert_eq!(
        export_mime,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );

    let ext = extension_for_export_mime(export_mime);
    assert_eq!(ext, ".docx");

    // The downloader would append .docx when saving
    let target = std::path::Path::new("/home/user/GoogleDrive/MyDocument");
    let with_ext = format!("{}{}", target.display(), ext);
    assert_eq!(with_ext, "/home/user/GoogleDrive/MyDocument.docx");
}

#[test]
fn discover_all_workspace_types() {
    let workspace_types = [
        ("application/vnd.google-apps.document", ".docx"),
        ("application/vnd.google-apps.spreadsheet", ".xlsx"),
        ("application/vnd.google-apps.presentation", ".pptx"),
        ("application/vnd.google-apps.drawing", ".png"),
    ];

    for (drive_mime, expected_ext) in workspace_types {
        assert!(
            is_workspace_document(drive_mime),
            "{drive_mime} should be workspace"
        );
        let export = workspace_export_mime(drive_mime).unwrap();
        assert_eq!(
            extension_for_export_mime(export),
            expected_ext,
            "extension for {drive_mime}"
        );
    }
}

// ── Upload pipeline ────────────────────────────────────────────────────────

#[test]
fn prepare_upload_for_small_file() {
    // A small text file (100 bytes)
    let file_len: u64 = 100;
    assert!(file_len < RESUMABLE_THRESHOLD);

    // Should use simple multipart upload (no chunking needed)
    let chunk_size = recommended_chunk_size(file_len);
    assert_eq!(chunk_size, file_len); // entire file in one chunk
    assert_eq!(chunk_count(file_len, chunk_size), 1);
}

#[test]
fn prepare_upload_for_large_file_with_chunking() {
    // A 20 MiB file
    let file_len: u64 = 20 * 1024 * 1024;

    assert!(file_len >= RESUMABLE_THRESHOLD);

    let chunk_size = recommended_chunk_size(file_len);
    assert_eq!(chunk_size, CHUNK_SIZE); // 5 MiB chunks
    assert!(is_valid_chunk_size(chunk_size));

    let count = chunk_count(file_len, chunk_size);
    assert_eq!(count, 4); // 20 MiB / 5 MiB = 4

    // Verify all chunk ranges
    let ranges: Vec<_> = (0..count)
        .map(|n| chunk_range(n, file_len, chunk_size).unwrap())
        .collect();

    assert_eq!(ranges[0], (0, CHUNK_SIZE - 1));
    assert_eq!(ranges[1], (CHUNK_SIZE, 2 * CHUNK_SIZE - 1));
    assert_eq!(ranges[2], (2 * CHUNK_SIZE, 3 * CHUNK_SIZE - 1));
    assert_eq!(ranges[3], (3 * CHUNK_SIZE, file_len - 1));

    // All ranges are contiguous and cover the full file
    assert_eq!(ranges[0].0, 0);
    assert_eq!(ranges.last().unwrap().1, file_len - 1);
}

#[test]
fn upload_chunk_alignment_validation() {
    // All chunk sizes in the upload pipeline must be multiples of 256 KiB
    assert!(is_valid_chunk_size(CHUNK_SIZE));
    assert!(is_valid_chunk_size(MIN_CHUNK_SIZE));
    assert!(!is_valid_chunk_size(CHUNK_SIZE + 1));
    assert!(!is_valid_chunk_size(0));

    // Chunk count edge cases
    assert_eq!(chunk_count(0, CHUNK_SIZE), 0);
    assert_eq!(chunk_count(1, CHUNK_SIZE), 1);
    assert_eq!(chunk_count(CHUNK_SIZE, CHUNK_SIZE), 1);
    assert_eq!(chunk_count(CHUNK_SIZE + 1, CHUNK_SIZE), 2);
}

#[test]
fn resumable_upload_thresholds_and_constants() {
    assert_eq!(RESUMABLE_THRESHOLD, 5 * 1024 * 1024);
    assert_eq!(CHUNK_SIZE, 5 * 1024 * 1024);
    assert_eq!(MIN_CHUNK_SIZE, 256 * 1024);
    assert_eq!(DL_MAX_RETRIES, 3);
    assert_eq!(UL_MAX_RETRIES, 3);
}

// ── Conflict detection scenarios ───────────────────────────────────────────

#[test]
fn conflict_both_modified_since_last_sync() {
    // Local file modified after last sync AND remote etag changed
    let conflict = detect_conflict(
        1700000000,           // current_local_mtime_ms
        Some(1699000000),     // cached_local_mtime_ms
        Some("\"old-etag\""), // cached_remote_etag
        "\"new-etag\"",       // current_remote_etag
    );
    assert!(conflict, "both sides modified → conflict");
}

#[test]
fn no_conflict_local_only_modified() {
    // Only local modified, remote unchanged
    let conflict = detect_conflict(
        1700000000,       // local is newer
        Some(1699000000), // stored is older
        Some("\"same-etag\""),
        "\"same-etag\"", // etag unchanged
    );
    assert!(
        !conflict,
        "only local changed → not a conflict (just upload)"
    );
}

#[test]
fn no_conflict_remote_only_modified() {
    // Only remote modified, local unchanged
    let conflict = detect_conflict(
        1699000000, // local = stored
        Some(1699000000),
        Some("\"old-etag\""),
        "\"new-etag\"", // remote changed
    );
    assert!(
        !conflict,
        "only remote changed → not a conflict (just download)"
    );
}

#[test]
fn no_conflict_neither_changed() {
    let conflict = detect_conflict(
        1700000000,
        Some(1700000000), // same mtime
        Some("\"same\""),
        "\"same\"", // same etag
    );
    assert!(!conflict);
}

#[test]
fn no_conflict_no_cached_etag() {
    // Can't detect remote change without a cached etag
    let conflict = detect_conflict(
        1700000000,
        Some(1699000000),
        None,           // no cached etag
        "\"new-etag\"", // current etag exists
    );
    assert!(!conflict);
}

#[test]
fn no_conflict_no_cached_mtime() {
    // Can't detect local change without a cached mtime
    // New local file that was never synced → no conflict
    let conflict = detect_conflict(
        1700000000,
        None,              // no cached mtime (new file)
        None,              // no cached etag
        "\"remote-etag\"", // file exists on remote
    );
    assert!(
        !conflict,
        "no cached mtime → can't determine local changed → no conflict"
    );
}

#[test]
fn no_conflict_new_file_local_only() {
    // New local file, does not exist on remote
    // Without cached data, can't determine changes
    let conflict = detect_conflict(
        1700000000, None, None, "", // empty etag (file doesn't exist on remote)
    );
    assert!(!conflict, "no cached data → cannot detect conflict");
}

#[test]
fn conflict_one_ms_difference() {
    // One millisecond difference in local mtime with remote change
    let conflict = detect_conflict(1700000001, Some(1700000000), Some("\"old\""), "\"new\"");
    assert!(conflict);
}

// ── Conflict copy naming ───────────────────────────────────────────────────

#[test]
fn conflict_copy_name_generation() {
    let path = std::path::Path::new("/home/user/report.pdf");
    let name = conflict_copy_name(path, "2026-05-05 14:30:00");
    assert!(name.starts_with("report (conflict copy "));
    assert!(name.ends_with("2026-05-05 14:30:00).pdf"));
}

#[test]
fn conflict_copy_name_no_extension() {
    let path = std::path::Path::new("/home/user/README");
    let name = conflict_copy_name(path, "2026-05-05 14:30:00");
    assert_eq!(name, "README (conflict copy 2026-05-05 14:30:00)");
}

#[test]
fn conflict_copy_name_hidden_file() {
    let path = std::path::Path::new("/home/user/.gitignore");
    let name = conflict_copy_name(path, "2026-05-05 14:30:00");
    assert_eq!(name, ".gitignore (conflict copy 2026-05-05 14:30:00)");
}

#[test]
fn conflict_copy_name_multiple_dots() {
    let path = std::path::Path::new("archive.tar.gz");
    let name = conflict_copy_name(path, "2026-05-05 14:30:00");
    assert_eq!(name, "archive.tar (conflict copy 2026-05-05 14:30:00).gz");
}

// ── MIME type guessing ─────────────────────────────────────────────────────

#[test]
fn guess_mime_common_types() {
    assert_eq!(guess_mime(std::path::Path::new("file.txt")), "text/plain");
    assert_eq!(
        guess_mime(std::path::Path::new("doc.pdf")),
        "application/pdf"
    );
    assert_eq!(guess_mime(std::path::Path::new("img.png")), "image/png");
    assert_eq!(guess_mime(std::path::Path::new("img.jpg")), "image/jpeg");
    assert_eq!(guess_mime(std::path::Path::new("img.jpeg")), "image/jpeg");
    assert_eq!(guess_mime(std::path::Path::new("video.mp4")), "video/mp4");
    assert_eq!(
        guess_mime(std::path::Path::new("doc.docx")),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );
}

#[test]
fn guess_mime_unknown_returns_octet_stream() {
    assert_eq!(
        guess_mime(std::path::Path::new("file.xyzzy")),
        "application/octet-stream"
    );
    assert_eq!(
        guess_mime(std::path::Path::new("Makefile")),
        "application/octet-stream"
    );
}

// ── Full sync simulation ───────────────────────────────────────────────────

/// Simulates the daemon's sync orchestrator walking through a full sync
/// lifecycle: discover files → classify (workspace vs regular) → resolve
/// conflicts → queue downloads.
#[test]
fn simulate_full_sync_lifecycle() {
    // Files discovered from Drive API
    let remote_files = vec![
        ("file-1", "report.pdf", "application/pdf", "\"etag-a\""),
        (
            "file-2",
            "budget.xlsx",
            "application/vnd.google-apps.spreadsheet",
            "\"etag-b\"",
        ),
        ("file-3", "photo.png", "image/png", "\"etag-c\""),
        ("file-4", "notes.txt", "text/plain", "\"etag-d\""),
    ];

    let mut download_queue: Vec<String> = Vec::new();
    let mut export_queue: Vec<(&str, &str)> = Vec::new(); // (file_id, export_mime)

    for (id, name, mime, _etag) in &remote_files {
        if is_workspace_document(mime) {
            let export_mime = workspace_export_mime(mime).unwrap();
            export_queue.push((id, export_mime));
        } else {
            download_queue.push(format!("{}/{}", id, name));
        }
    }

    // Exactly 1 workspace doc among the 4 files (spreadsheet)
    assert_eq!(export_queue.len(), 1);
    assert_eq!(download_queue.len(), 3); // pdf, png, txt → direct download

    // Export mime types are correct
    assert_eq!(
        export_queue[0].1,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
}

#[test]
fn simulate_conflict_detection_during_sync() {
    // Simulate the daemon comparing local file state with remote state.
    // The detect_conflict function requires: current local mtime, cached local mtime,
    // cached remote etag, and current remote etag.
    // Conflict only when BOTH local changed AND remote changed.

    let files = vec![
        // (name, current_local_mtime, cached_mtime, cached_etag, current_etag, expect_conflict)
        (
            "report.pdf",
            1700000000_i64,
            Some(1699000000_i64),
            Some("\"a\""),
            "\"a\"",
            false,
        ), // local modified only
        (
            "data.csv",
            1700000000_i64,
            Some(1699000000_i64),
            Some("\"b\""),
            "\"c\"",
            true,
        ), // both modified
        (
            "notes.txt",
            1699000000_i64,
            Some(1699000000_i64),
            Some("\"d\""),
            "\"d\"",
            false,
        ), // neither changed
        (
            "remote.txt",
            1699000000_i64,
            Some(1699000000_i64),
            Some("\"e\""),
            "\"f\"",
            false,
        ), // remote only changed
    ];

    let mut conflicts = Vec::new();
    for (name, current_mtime, cached_mtime, cached_etag, current_etag, _) in &files {
        if detect_conflict(*current_mtime, *cached_mtime, *cached_etag, current_etag) {
            conflicts.push(*name);
        }
    }

    assert_eq!(conflicts.len(), 1);
    assert!(conflicts.contains(&"data.csv"));
}
