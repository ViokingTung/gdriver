import FileProvider
import Foundation

/// Manages communication with the `gdriver-daemon` process.
///
/// Uses the daemon's Unix-socket IPC (JSON-RPC 2.0 over NDJSON) for file
/// metadata and content operations. The socket path is resolved from the
/// daemon's standard location: `$XDG_RUNTIME_DIR/gdriver.sock` or
/// `$TMPDIR/gdriver.sock`.
///
/// In the future, an XPC channel can be added for lifecycle coordination
/// alongside the Unix-socket data channel.
actor DaemonConnection {

    // MARK: - Properties

    private var socketPath: String {
        // macOS: use $TMPDIR/gdriver.sock
        if let tmp = ProcessInfo.processInfo.environment["TMPDIR"] {
            return "\(tmp)/gdriver.sock"
        }
        return "/tmp/gdriver.sock"
    }

    private var connection: FileHandle?
    private var requestId: UInt64 = 0

    // MARK: - Connection

    func connect() throws {
        guard connection == nil else { return }

        let path = socketPath
        guard FileManager.default.fileExists(atPath: path) else {
            throw GDriverError.daemonNotRunning
        }

        let fd = open(path, O_RDWR)
        guard fd >= 0 else {
            throw GDriverError.connectionFailed(errno: errno)
        }

        connection = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        NSLog("gDriver: connected to daemon at \(path)")
    }

    func disconnect() {
        connection?.closeFile()
        connection = nil
    }

    // MARK: - RPC helpers

    private func nextRequestId() -> UInt64 {
        requestId += 1
        return requestId
    }

    private func sendRPC(method: String, params: [String: Any]) throws -> [String: Any] {
        let handle = try ensureConnection()

        let req: [String: Any] = [
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": nextRequestId(),
        ]

        let data = try JSONSerialization.data(withJSONObject: req)
        var line = String(data: data, encoding: .utf8) ?? ""
        line += "\n"

        guard let lineData = line.data(using: .utf8) else {
            throw GDriverError.serializationFailed
        }

        try handle.write(contentsOf: lineData)

        // Read the response line.
        guard let responseLine = try readLine(from: handle) else {
            throw GDriverError.noResponse
        }

        guard let responseData = responseLine.data(using: .utf8),
              let response = try JSONSerialization.jsonObject(with: responseData) as? [String: Any]
        else {
            throw GDriverError.invalidResponse
        }

        if let error = response["error"] as? [String: Any] {
            let message = error["message"] as? String ?? "unknown error"
            throw GDriverError.rpcError(message)
        }

        guard let result = response["result"] as? [String: Any] else {
            throw GDriverError.invalidResponse
        }

        return result
    }

    private func ensureConnection() throws -> FileHandle {
        if connection == nil {
            try connect()
        }
        guard let conn = connection else {
            throw GDriverError.connectionFailed(errno: 0)
        }
        return conn
    }

    private func readLine(from handle: FileHandle) throws -> String? {
        var buffer = Data()
        let maxBytes = 65536

        for _ in 0..<maxBytes {
            let chunk = try handle.read(upToCount: 1)
            guard let byte = chunk, byte.count == 1 else {
                return buffer.isEmpty ? nil : String(data: buffer, encoding: .utf8)
            }

            if byte[0] == UInt8(ascii: "\n") {
                return String(data: buffer, encoding: .utf8)
            }
            buffer.append(byte)
        }

        return String(data: buffer, encoding: .utf8)
    }

    // MARK: - FileProvider operations

    /// Fetch metadata for a single FileProvider item.
    func fetchItem(identifier: NSFileProviderItemIdentifier) async throws -> NSFileProviderItem {
        let path = identifierToPath(identifier)
        let result = try sendRPC(
            method: "fp.get_item",
            params: ["path": path]
        )

        return try parseItem(from: result)
    }

    /// Fetch contents of a file, returning a local URL and updated item.
    func fetchContents(
        identifier: NSFileProviderItemIdentifier,
        progress: Progress
    ) async throws -> (URL, NSFileProviderItem) {
        let path = identifierToPath(identifier)
        let result = try sendRPC(
            method: "fp.fetch_contents",
            params: ["path": path]
        )

        guard let localPath = result["local_path"] as? String else {
            throw GDriverError.invalidResponse
        }

        let url = URL(fileURLWithPath: localPath)
        let item = try parseItem(from: result)

        progress.completedUnitCount = 100
        return (url, item)
    }

    /// Enumerate children of a container.
    func enumerateChildren(
        of identifier: NSFileProviderItemIdentifier
    ) async throws -> [NSFileProviderItem] {
        let path = identifierToPath(identifier)
        let result = try sendRPC(
            method: "fp.list_children",
            params: ["path": path]
        )

        guard let items = result["items"] as? [[String: Any]] else {
            throw GDriverError.invalidResponse
        }

        return items.compactMap { try? parseItem(from: $0) }
    }

    /// Create a new item in Drive.
    func createItem(
        template: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?
    ) async throws -> NSFileProviderItem {
        var params: [String: Any] = [
            "name": template.filename,
            "parent": identifierToPath(template.parentItemIdentifier),
        ]

        if template.contentType?.conforms(to: .directory) == true {
            params["is_folder"] = true
        }

        if let url = url {
            params["local_path"] = url.path
        }

        let result = try sendRPC(method: "fp.create_item", params: params)
        return try parseItem(from: result)
    }

    /// Modify an existing item.
    func modifyItem(
        item: NSFileProviderItem,
        changedFields: NSFileProviderItemFields,
        contents url: URL?
    ) async throws -> NSFileProviderItem {
        var params: [String: Any] = [
            "path": identifierToPath(item.itemIdentifier),
        ]

        if changedFields.contains(.filename), let name = item.filename {
            params["new_name"] = name
        }
        if changedFields.contains(.parentItemIdentifier) {
            params["new_parent"] = identifierToPath(item.parentItemIdentifier)
        }
        if let url = url {
            params["local_path"] = url.path
        }

        let result = try sendRPC(method: "fp.modify_item", params: params)
        return try parseItem(from: result)
    }

    /// Delete an item.
    func deleteItem(identifier: NSFileProviderItemIdentifier) async throws {
        let path = identifierToPath(identifier)
        _ = try sendRPC(
            method: "fp.delete_item",
            params: ["path": path]
        )
    }

    // MARK: - Parsing

    private func identifierToPath(_ identifier: NSFileProviderItemIdentifier) -> String {
        if identifier == .rootContainer {
            return "/"
        }
        // The identifier is "account_id:file_id". Convert to the local
        // path relative to the mount point.
        return identifier.rawValue
    }

    private func parseItem(from dict: [String: Any]) throws -> FileProviderItem {
        guard let name = dict["name"] as? String else {
            throw GDriverError.invalidResponse
        }

        let fileId = dict["file_id"] as? String ?? ""
        let accountId = dict["account_id"] as? String ?? ""
        let identifier = NSFileProviderItemIdentifier("\(accountId):\(fileId)")

        let isFolder = (dict["mime_type"] as? String) == "application/vnd.google-apps.folder"
        let size = (dict["size"] as? NSNumber)?.int64Value ?? 0
        let mtimeMs = (dict["modified_time"] as? NSNumber)?.doubleValue ?? 0
        let mtime = Date(timeIntervalSince1970: mtimeMs / 1000.0)
        let syncState = dict["sync_state"] as? String ?? "cloud_only"

        // Parent: if dict has parent_file_id, use it; otherwise root.
        let parentId: NSFileProviderItemIdentifier
        if let parentFileId = dict["parent_file_id"] as? String, !parentFileId.isEmpty {
            parentId = NSFileProviderItemIdentifier("\(accountId):\(parentFileId)")
        } else {
            parentId = .rootContainer
        }

        // Determine capabilities based on sync state.
        var capabilities: NSFileProviderItemCapabilities = [.allowsReading]
        if isFolder {
            capabilities.insert(.allowsAddingSubItems)
            capabilities.insert(.allowsContentEnumerating)
        }
        if !isFolder && syncState != "cloud_only" {
            capabilities.insert(.allowsWriting)
        }
        capabilities.insert(.allowsRenaming)
        capabilities.insert(.allowsDeleting)
        capabilities.insert(.allowsTrashing)

        // Offline / pinned state.
        let isUploaded = syncState == "synced"
        let isDownloaded = syncState == "cached" || syncState == "offline" || isUploaded
        let isMostRecentVersionDownloaded = isDownloaded
        let isUploading = syncState == "modified"

        let contentType: UTType = isFolder ? .folder : UTType(filenameExtension: (name as NSString).pathExtension) ?? .data

        return FileProviderItem(
            identifier: identifier,
            filename: name,
            contentType: contentType,
            parentIdentifier: parentId,
            documentSize: NSNumber(value: size),
            creationDate: mtime,
            contentModificationDate: mtime,
            capabilities: capabilities,
            isUploaded: isUploaded,
            isDownloaded: isDownloaded,
            isMostRecentVersionDownloaded: isMostRecentVersionDownloaded,
            isUploading: isUploading,
            isShared: dict["is_shared"] as? Bool ?? false
        )
    }
}

// MARK: - Error types

enum GDriverError: LocalizedError {
    case daemonNotRunning
    case connectionFailed(errno: Int32)
    case serializationFailed
    case noResponse
    case invalidResponse
    case rpcError(String)
    case notFound

    var errorDescription: String? {
        switch self {
        case .daemonNotRunning:
            return "The gDriver daemon is not running."
        case .connectionFailed(let e):
            return "Failed to connect to daemon (errno: \(e))."
        case .serializationFailed:
            return "Failed to serialize RPC request."
        case .noResponse:
            return "No response from daemon."
        case .invalidResponse:
            return "Invalid response from daemon."
        case .rpcError(let msg):
            return "Daemon RPC error: \(msg)"
        case .notFound:
            return "File not found."
        }
    }

    var errorCode: Int {
        switch self {
        case .notFound: return 404
        default: return -1
        }
    }
}
