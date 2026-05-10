import FileProvider

/// Entry point for the FileProvider Extension.
///
/// The system creates an instance of `FileProviderExtension` and uses it
/// to serve the domain `~/Library/CloudStorage/GoogleDrive-{account}/`.

@main
struct GDriverFileProviderExtension {
    static func main() throws {
        let domain = NSFileProviderDomain(
            identifier: NSFileProviderDomainIdentifier("com.gdriver.fileprovider"),
            displayName: "Google Drive"
        )

        let manager = NSFileProviderManager(for: domain)
        guard let manager = manager else {
            fatalError("FileProvider domain not found. Ensure the extension is registered.")
        }

        // The actual extension instance is created by the FileProvider
        // framework when needed. This main entry point registers the
        // domain if it doesn't already exist.
        NSFileProviderManager.add(domain) { error in
            if let error = error as NSError? {
                // NSCocoaErrorDomain, file exists error is expected on
                // subsequent launches.
                if error.domain == NSCocoaErrorDomain && error.code == 516 {
                    // Domain already registered — OK.
                } else {
                    NSLog("gDriver: failed to add FileProvider domain: \(error)")
                }
            }
        }
    }
}
