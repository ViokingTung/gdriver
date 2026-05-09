# gDriver Windows Shell Extension

A COM DLL that provides Windows Explorer integration for gDriver:

- **Icon Overlays**: Shows sync status icons on files
  - Gray cloud: File is only in the cloud
  - Blue spinning arrow: File is being synced
  - Green checkmark: File is synced/cached/offline
  - Red X: Sync error
  - Blue up arrow: File is being uploaded

- **Context Menu**: Right-click menu items for gDriver files
  - Available offline: Mark file for offline access
  - Online only: Remove offline access
  - Copy Drive link: Copy share link to clipboard
  - View in Drive web: Open in browser
  - Share: Open sharing dialog

## Prerequisites

- Windows 10/11
- Rust toolchain (rustup)
- gdriver-daemon running

## Building

```bash
cargo build --release
```

The DLL will be at `target/release/gdriver_shell.dll`.

## Installation

1. Copy `gdriver_shell.dll` to `%PROGRAMFILES%\gDriver\`
2. Run `register.reg` as administrator (double-click or `regedit /s register.reg`)
3. Restart Explorer (`taskkill /f /im explorer.exe && start explorer.exe`)

## Uninstallation

1. Run `unregister.reg` as administrator
2. Restart Explorer
3. Delete the DLL file

## IPC Communication

The extension communicates with gdriver-daemon over Named Pipes using JSON-RPC 2.0:

- Pipe path: `\\.\pipe\gdriver`
- Methods:
  - `fs.getSyncState(path)` - Get sync state for a file
  - `fs.setOffline(path, enabled)` - Set offline availability
  - `fs.getShareLink(path)` - Get Google Drive share link

## Architecture

```
┌─────────────────────┐
│   Windows Explorer   │
│                      │
│  ┌────────────────┐  │
│  │ Shell Extension │  │
│  │  (COM DLL)     │  │
│  └───────┬────────┘  │
└──────────┼───────────┘
           │ Named Pipe
           │ JSON-RPC 2.0
┌──────────▼───────────┐
│   gdriver-daemon     │
│   (Rust process)     │
└──────────────────────┘
```

## Registry Entries

The shell extension registers itself under:

- `HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\gDriver`
- `HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriver*`

Each overlay has its own CLSID for proper Windows integration.

## Troubleshooting

1. **Icons not showing**: Windows limits icon overlays to 15. Check if other apps (OneDrive, Dropbox) are using all slots.

2. **Context menu not appearing**: Ensure the DLL is properly registered and Explorer was restarted.

3. **IPC connection failed**: Make sure gdriver-daemon is running and the Named Pipe is available at `\\.\pipe\gdriver`.
