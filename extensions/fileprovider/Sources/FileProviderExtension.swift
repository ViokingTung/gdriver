import FileProvider
import Foundation

/// Main FileProvider extension implementation.
///
/// Implements `NSFileProviderReplicatedExtension` to expose Google Drive
/// files through the macOS FileProvider framework. Communicates with the
/// `gdriver-daemon` process via XPC / Unix-socket IPC for file metadata
/// and content.
class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension {

    // MARK: - Properties

    private let domain: NSFileProviderDomain
    private let daemon: DaemonConnection

    // MARK: - Initialization

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        self.daemon = DaemonConnection()
        super.init()
        NSLog("gDriver: FileProvider extension initialized for domain \(domain.identifier)")
    }

    // MARK: - NSFileProviderReplicatedExtension

    func invalidate() {
        Task { await daemon.disconnect() }
        NSLog("gDriver: FileProvider extension invalidated")
    }

    // MARK: - Item lookup

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)

        Task {
            do {
                let item = try await daemon.fetchItem(identifier: identifier)
                completionHandler(item, nil)
            } catch {
                if (error as NSError).domain == "GDriver" && (error as NSError).code == 404 {
                    completionHandler(nil, NSFileProviderError(.noSuchItem))
                } else {
                    completionHandler(nil, error)
                }
            }
            progress.completedUnitCount = 1
        }

        return progress
    }

    // MARK: - Directory enumeration

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        return FileProviderEnumerator(
            containerIdentifier: containerItemIdentifier,
            daemon: daemon
        )
    }

    // MARK: - Content fetching

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion?,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 100)

        Task {
            do {
                let (fileURL, updatedItem) = try await daemon.fetchContents(
                    identifier: itemIdentifier,
                    progress: progress
                )
                completionHandler(fileURL, updatedItem, nil)
            } catch {
                completionHandler(nil, nil, error)
            }
        }

        return progress
    }

    // MARK: - Modifications

    func createItem(
        basedOn itemTemplate: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?,
        options: NSFileProviderCreateItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)

        Task {
            do {
                let createdItem = try await daemon.createItem(
                    template: itemTemplate,
                    fields: fields,
                    contents: url
                )
                completionHandler(createdItem, [], false, nil)
            } catch {
                completionHandler(nil, [], false, error)
            }
            progress.completedUnitCount = 1
        }

        return progress
    }

    func modifyItem(
        _ item: NSFileProviderItem,
        baseVersion version: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields,
        contents newContents: URL?,
        options: NSFileProviderModifyItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)

        Task {
            do {
                let modifiedItem = try await daemon.modifyItem(
                    item: item,
                    changedFields: changedFields,
                    contents: newContents
                )
                completionHandler(modifiedItem, [], false, nil)
            } catch {
                completionHandler(nil, [], false, error)
            }
            progress.completedUnitCount = 1
        }

        return progress
    }

    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)

        Task {
            do {
                try await daemon.deleteItem(identifier: identifier)
                completionHandler(nil)
            } catch {
                completionHandler(error)
            }
            progress.completedUnitCount = 1
        }

        return progress
    }
}
