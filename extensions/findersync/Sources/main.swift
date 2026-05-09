import FinderSync

/// Entry point for the Finder Sync Extension.
///
/// The system loads `GDriverFinderSync` as the principal class (declared in
/// `Info.plist` under `NSExtensionPrincipalClass`) and initialises it to
/// begin monitoring the Google Drive mount point.
@main
struct GDriverFinderSyncMain {
    static func main() {
        // The Finder Sync extension infrastructure manages the lifecycle
        // automatically.  We just need to register the principal class.
        FIFinderSyncController.default().setEnabled(true)
    }
}
