import FileProvider
import Foundation

/// Enumerates the children of a Google Drive folder for the FileProvider
/// framework.
///
/// Communicates with the `gdriver-daemon` to fetch directory contents on
/// demand, with support for incremental / paginated enumeration via the
/// `NSFileProviderEnumerator` protocol.
class FileProviderEnumerator: NSObject, NSFileProviderEnumerator {

    private let containerIdentifier: NSFileProviderItemIdentifier
    private let daemon: DaemonConnection
    private var invalidated = false

    init(containerIdentifier: NSFileProviderItemIdentifier, daemon: DaemonConnection) {
        self.containerIdentifier = containerIdentifier
        self.daemon = daemon
        super.init()
    }

    func invalidate() {
        invalidated = true
    }

    func enumerateItems(
        for observer: NSFileProviderEnumerationObserver,
        startingAt page: NSFileProviderPage
    ) {
        guard !invalidated else {
            observer.finishEnumerating(upTo: nil)
            return
        }

        Task {
            do {
                let items = try await daemon.enumerateChildren(of: containerIdentifier)
                observer.didEnumerate(items)

                // No pagination in the initial implementation — signal
                // completion with `upTo: nil`.
                observer.finishEnumerating(upTo: nil)
            } catch {
                NSLog("gDriver: enumeration error for \(containerIdentifier): \(error)")
                observer.finishEnumeratingWithError(error)
            }
        }
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from anchor: NSFileProviderSyncAnchor
    ) {
        // TODO: Implement incremental change enumeration based on the
        // sync engine's page token. For the initial implementation, signal
        // that the anchor is up-to-date (no changes) so the system does
        // a full re-enumeration periodically.
        observer.finishEnumeratingChanges(
            upTo: anchor,
            moreComing: false
        )
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        // Use a timestamp-based anchor for now.
        let now = Date()
        let data = withUnsafeBytes(of: now.timeIntervalSince1970) { Data($0) }
        let anchor = NSFileProviderSyncAnchor(data)
        completionHandler(anchor)
    }
}
