import FileProvider
import UniformTypeIdentifiers

/// Represents a single file or folder in the FileProvider domain.
///
/// Conforms to `NSFileProviderItem` with the full set of properties
/// that Finder uses to display the item (name, size, date, capabilities,
/// sync status).
class FileProviderItem: NSObject, NSFileProviderItem {

    // MARK: - Core identity

    init(
        itemIdentifier: NSFileProviderItemIdentifier,
        parentItemIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        contentType: UTType,
        documentSize: NSNumber?,
        creationDate: Date?,
        contentModificationDate: Date?,
        capabilities: NSFileProviderItemCapabilities,
        isUploaded: Bool,
        isDownloaded: Bool,
        isMostRecentVersionDownloaded: Bool,
        isUploading: Bool,
        isShared: Bool
    ) {
        self.itemIdentifier = itemIdentifier
        self.parentItemIdentifier = parentItemIdentifier
        self.filename = filename
        self.contentType = contentType
        self.documentSize = documentSize
        self.creationDate = creationDate
        self.contentModificationDate = contentModificationDate
        self.capabilities = capabilities
        self.isUploaded = isUploaded
        self.isDownloaded = isDownloaded
        self.isMostRecentVersionDownloaded = isMostRecentVersionDownloaded
        self.isUploading = isUploading
        self.isShared = isShared
        super.init()
    }

    let itemIdentifier: NSFileProviderItemIdentifier
    let parentItemIdentifier: NSFileProviderItemIdentifier
    let filename: String
    let contentType: UTType

    // MARK: - Metadata

    let documentSize: NSNumber?
    let creationDate: Date?
    let contentModificationDate: Date?

    // MARK: - Capabilities

    let capabilities: NSFileProviderItemCapabilities

    // MARK: - Sync state

    var isUploaded: Bool
    var isDownloaded: Bool
    var isMostRecentVersionDownloaded: Bool
    var isUploading: Bool

    // MARK: - Sharing

    var isShared: Bool

    // MARK: - Type information

    var isFolder: Bool {
        contentType.conforms(to: .directory)
    }

    // MARK: - Version

    var itemVersion: NSFileProviderItemVersion {
        // Use modification date as a simple version identifier.
        let mtime = contentModificationDate ?? Date()
        let data = withUnsafeBytes(of: mtime.timeIntervalSince1970) { Data($0) }
        return NSFileProviderItemVersion(contentVersion: data, metadataVersion: data)
    }

    // MARK: - Offline / downloading badge

    /// Show a cloud icon when the file is not downloaded.
    var isDownloadRequested: Bool {
        !isDownloaded
    }

    // MARK: - User-visible info

    /// Show user who owns the file (for shared files).
    var ownerNameComponents: PersonNameComponents? {
        guard isShared else { return nil }
        var comps = PersonNameComponents()
        comps.nickname = "Shared"
        return comps
    }
}
