import Cocoa
import FinderSync
import os.log

/// Badge identifiers registered with `FIFinderSyncController`.
private let badgeCloud = "com.gdriver.badge.cloud"
private let badgeSyncing = "com.gdriver.badge.syncing"
private let badgeSynced = "com.gdriver.badge.synced"
private let badgeError = "com.gdriver.badge.error"

/// Maps daemon sync-state strings to badge identifiers.
private let badgeForState: [String: String] = [
    "cloud_only": badgeCloud,
    "syncing": badgeSyncing,
    "synced": badgeSynced,
    "cached": badgeSynced,
    "offline": badgeSynced,
    "error": badgeError,
]

private let log = OSLog(subsystem: "com.gdriver.findersync", category: "FinderSync")

final class GDriverFinderSync: FIFinderSync {

    private let daemon = DaemonConnection()
    private let monitoredRoot: URL

    override init() {
        monitoredRoot = URL(fileURLWithPath: ("~/GoogleDrive" as NSString).expandingTildeInPath)
        super.init()

        // Register badge images. The extension bundle must include:
        //   badge_cloud.png, badge_syncing.png, badge_synced.png, badge_error.png
        let controller = FIFinderSyncController.default()
        if let cloudImg = Bundle.main.image(forResource: "badge_cloud") {
            controller.setBadgeImage(cloudImg, label: "Cloud only", forBadgeIdentifier: badgeCloud)
        }
        if let syncingImg = Bundle.main.image(forResource: "badge_syncing") {
            controller.setBadgeImage(syncingImg, label: "Syncing", forBadgeIdentifier: badgeSyncing)
        }
        if let syncedImg = Bundle.main.image(forResource: "badge_synced") {
            controller.setBadgeImage(syncedImg, label: "Synced", forBadgeIdentifier: badgeSynced)
        }
        if let errorImg = Bundle.main.image(forResource: "badge_error") {
            controller.setBadgeImage(errorImg, label: "Error", forBadgeIdentifier: badgeError)
        }

        // Monitor the Google Drive mount point.
        controller.directoryURLs = [monitoredRoot]
        os_log(.info, log: log, "Finder Sync initialized, monitoring %{public}@", monitoredRoot.path)
    }

    // MARK: - Badge overlay

    override func requestBadgeIdentifier(for url: URL) {
        // Only process files under the monitored root.
        guard url.path.hasPrefix(monitoredRoot.path) else { return }

        Task {
            do {
                let state = try await daemon.getSyncState(path: url.path)
                let badge = badgeForState[state] ?? ""
                FIFinderSyncController.default().setBadgeIdentifier(badge, for: url)
            } catch {
                // Don't badge files we can't query.
            }
        }
    }

    // MARK: - Context menu

    override func menu(for menuKind: FIMenuKind) -> NSMenu? {
        // We only show menus for selected items in Finder.
        guard menuKind == .contextualMenuForItems else { return nil }

        let items = FIFinderSyncController.default().selectedItemURLs() ?? []
        // Single-file actions only.
        guard items.count == 1, let url = items.first else { return nil }
        guard url.path.hasPrefix(monitoredRoot.path) else { return nil }

        let menu = NSMenu(title: "")
        let path = url.path

        // "gDrive" submenu.
        let submenu = NSMenu()
        let rootItem = NSMenuItem(title: "gDrive", action: nil, keyEquivalent: "")
        rootItem.submenu = submenu

        // We cannot call async from a synchronous menu callback, so we
        // pre-fetch state when the menu builds.  Use a semaphore with a
        // short timeout so Finder doesn't block indefinitely.
        var currentState = ""
        let sem = DispatchSemaphore(value: 0)
        Task {
            do {
                currentState = try await daemon.getSyncState(path: path)
            } catch {
                // Keep empty state.
            }
            sem.signal()
        }
        _ = sem.wait(timeout: .now() + 1.0)

        // Available offline / Online only toggle.
        if currentState == "cloud_only" || currentState == "syncing" {
            let item = NSMenuItem(
                title: NSLocalizedString("Make available offline", comment: ""),
                action: #selector(makeOffline(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = path
            submenu.addItem(item)
        } else if currentState == "synced" || currentState == "cached" || currentState == "offline" {
            let item = NSMenuItem(
                title: NSLocalizedString("Free up space", comment: ""),
                action: #selector(onlineOnly(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = path
            submenu.addItem(item)
        }

        submenu.addItem(NSMenuItem.separator())

        // Copy link.
        let copyLinkItem = NSMenuItem(
            title: NSLocalizedString("Copy link", comment: ""),
            action: #selector(copyLink(_:)),
            keyEquivalent: ""
        )
        copyLinkItem.target = self
        copyLinkItem.representedObject = path
        submenu.addItem(copyLinkItem)

        // View in Drive.
        let viewItem = NSMenuItem(
            title: NSLocalizedString("View in Drive", comment: ""),
            action: #selector(viewInDrive(_:)),
            keyEquivalent: ""
        )
        viewItem.target = self
        viewItem.representedObject = path
        submenu.addItem(viewItem)

        // Share.
        let shareItem = NSMenuItem(
            title: NSLocalizedString("Share", comment: ""),
            action: #selector(share(_:)),
            keyEquivalent: ""
        )
        shareItem.target = self
        shareItem.representedObject = path
        submenu.addItem(shareItem)

        menu.addItem(rootItem)
        return menu
    }

    // MARK: - Actions

    @objc private func makeOffline(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        Task {
            do {
                try await daemon.setOffline(path: path, enabled: true)
            } catch {
                os_log(.error, log: log, "setOffline failed: %{public}@", error.localizedDescription)
            }
        }
    }

    @objc private func onlineOnly(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        Task {
            do {
                try await daemon.setOffline(path: path, enabled: false)
            } catch {
                os_log(.error, log: log, "setOffline failed: %{public}@", error.localizedDescription)
            }
        }
    }

    @objc private func copyLink(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        Task {
            do {
                let url = try await daemon.getShareLink(path: path)
                let pasteboard = NSPasteboard.general
                pasteboard.clearContents()
                pasteboard.setString(url, forType: .string)
            } catch {
                os_log(.error, log: log, "getShareLink failed: %{public}@", error.localizedDescription)
            }
        }
    }

    @objc private func viewInDrive(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        Task {
            do {
                let url = try await daemon.getShareLink(path: path)
                NSWorkspace.shared.open(URL(string: url)!)
            } catch {
                os_log(.error, log: log, "viewInDrive failed: %{public}@", error.localizedDescription)
            }
        }
    }

    @objc private func share(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        Task {
            do {
                let url = try await daemon.getShareLink(path: path)
                let shareURL = url.replacingOccurrences(of: "/view", with: "/edit#sharing")
                NSWorkspace.shared.open(URL(string: shareURL)!)
            } catch {
                os_log(.error, log: log, "share failed: %{public}@", error.localizedDescription)
            }
        }
    }
}
