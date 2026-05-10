//! Download-related types and helpers.

use std::path::Path;

/// Maximum number of retry attempts before logging a persistent error.
pub const MAX_RETRIES: i32 = 3;

/// File extensions for Google Workspace documents that must be exported
/// rather than downloaded directly.
pub const WORKSPACE_EXPORT_FORMATS: &[(&str, &str)] = &[
    (
        "application/vnd.google-apps.document",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        "application/vnd.google-apps.spreadsheet",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        "application/vnd.google-apps.presentation",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("application/vnd.google-apps.drawing", "image/png"),
];

/// Returns the export MIME type for a Google Workspace document, or `None`
/// if the file is a regular file (downloadable directly).
pub fn workspace_export_mime(drive_mime_type: &str) -> Option<&'static str> {
    WORKSPACE_EXPORT_FORMATS
        .iter()
        .find(|(from, _)| *from == drive_mime_type)
        .map(|(_, to)| *to)
}

/// Returns `true` when the MIME type represents a Google Workspace document
/// that must be exported.
pub fn is_workspace_document(mime_type: &str) -> bool {
    workspace_export_mime(mime_type).is_some()
}

/// Build a temp file path alongside the target path.
///
/// `path` → `{path}.gdriver-tmp`
pub fn temp_download_path(target: &Path) -> String {
    format!("{}.gdriver-tmp", target.display())
}

/// Build a file extension for the workspace export format.
///
/// E.g. `application/pdf` → `.pdf`.
pub fn extension_for_export_mime(export_mime: &str) -> &'static str {
    match export_mime {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => ".xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => ".pptx",
        "image/png" => ".png",
        "text/plain" => ".txt",
        "application/pdf" => ".pdf",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constants ──────────────────────────────────────────────────────────

    #[test]
    fn max_retries_is_positive() {
        const { assert!(MAX_RETRIES > 0) };
    }

    #[test]
    fn workspace_export_formats_covers_all_workspace_types() {
        let types: Vec<&str> = WORKSPACE_EXPORT_FORMATS
            .iter()
            .map(|(from, _)| *from)
            .collect();
        assert!(types.contains(&"application/vnd.google-apps.document"));
        assert!(types.contains(&"application/vnd.google-apps.spreadsheet"));
        assert!(types.contains(&"application/vnd.google-apps.presentation"));
        assert!(types.contains(&"application/vnd.google-apps.drawing"));
    }

    // ── Workspace detection ────────────────────────────────────────────────

    #[test]
    fn google_doc_is_workspace() {
        assert!(is_workspace_document(
            "application/vnd.google-apps.document"
        ));
    }

    #[test]
    fn google_sheet_is_workspace() {
        assert!(is_workspace_document(
            "application/vnd.google-apps.spreadsheet"
        ));
    }

    #[test]
    fn google_slides_is_workspace() {
        assert!(is_workspace_document(
            "application/vnd.google-apps.presentation"
        ));
    }

    #[test]
    fn google_drawing_is_workspace() {
        assert!(is_workspace_document("application/vnd.google-apps.drawing"));
    }

    #[test]
    fn regular_file_is_not_workspace() {
        assert!(!is_workspace_document("text/plain"));
        assert!(!is_workspace_document("application/pdf"));
        assert!(!is_workspace_document("image/png"));
        assert!(!is_workspace_document("application/vnd.google-apps.folder"));
        assert!(!is_workspace_document(""));
    }

    #[test]
    fn workspace_export_mime_google_doc() {
        assert_eq!(
            workspace_export_mime("application/vnd.google-apps.document"),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
    }

    #[test]
    fn workspace_export_mime_google_sheet() {
        assert_eq!(
            workspace_export_mime("application/vnd.google-apps.spreadsheet"),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
        );
    }

    #[test]
    fn workspace_export_mime_google_slides() {
        assert_eq!(
            workspace_export_mime("application/vnd.google-apps.presentation"),
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        );
    }

    #[test]
    fn workspace_export_mime_google_drawing() {
        assert_eq!(
            workspace_export_mime("application/vnd.google-apps.drawing"),
            Some("image/png")
        );
    }

    #[test]
    fn workspace_export_mime_unknown() {
        assert_eq!(workspace_export_mime("text/plain"), None);
        assert_eq!(workspace_export_mime(""), None);
    }

    // ── Temp download path ─────────────────────────────────────────────────

    #[test]
    fn temp_download_path_simple() {
        let path = Path::new("/tmp/report.pdf");
        assert_eq!(temp_download_path(path), "/tmp/report.pdf.gdriver-tmp");
    }

    #[test]
    fn temp_download_path_no_extension() {
        let path = Path::new("/tmp/README");
        assert_eq!(temp_download_path(path), "/tmp/README.gdriver-tmp");
    }

    #[test]
    fn temp_download_path_hidden_file() {
        let path = Path::new("/tmp/.config");
        assert_eq!(temp_download_path(path), "/tmp/.config.gdriver-tmp");
    }

    // ── Export extension ───────────────────────────────────────────────────

    #[test]
    fn extension_for_docx() {
        assert_eq!(
            extension_for_export_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            ".docx"
        );
    }

    #[test]
    fn extension_for_xlsx() {
        assert_eq!(
            extension_for_export_mime(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            ),
            ".xlsx"
        );
    }

    #[test]
    fn extension_for_pptx() {
        assert_eq!(
            extension_for_export_mime(
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            ),
            ".pptx"
        );
    }

    #[test]
    fn extension_for_png() {
        assert_eq!(extension_for_export_mime("image/png"), ".png");
    }

    #[test]
    fn extension_for_pdf() {
        assert_eq!(extension_for_export_mime("application/pdf"), ".pdf");
    }

    #[test]
    fn extension_for_txt() {
        assert_eq!(extension_for_export_mime("text/plain"), ".txt");
    }

    #[test]
    fn extension_for_unknown() {
        assert_eq!(extension_for_export_mime("application/octet-stream"), "");
        assert_eq!(extension_for_export_mime(""), "");
    }
}
