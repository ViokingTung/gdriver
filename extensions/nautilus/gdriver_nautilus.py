"""Nautilus extension for gDriver — sync state emblems and context menus.

Provides:
- Emblem overlays showing file sync state (cloud-only, syncing, synced, error)
- Right-click menu items for Drive operations (offline, share link, etc.)

Requires: nautilus-python, GObject Introspection (GI), gdriver-daemon running.
"""

import os
import subprocess
import urllib.parse
from typing import Any, Dict, List, Optional

import gi

gi.require_version("Nautilus", "4.0")
from gi.repository import GObject, Nautilus  # noqa: E402

from gdriver_ipc import IpcClient, get_share_link, get_sync_state, set_offline

# ─── Constants ──────────────────────────────────────────────────────────────

EMBLEM_CLOUD = "emblem-gdriver-cloud"
EMBLEM_SYNCING = "emblem-gdriver-syncing"
EMBLEM_SYNCED = "emblem-gdriver-synced"
EMBLEM_ERROR = "emblem-gdriver-error"

# SyncState → emblem mapping (matches the Rust enum variants).
EMBLEM_MAP: Dict[str, str] = {
    "cloud_only": EMBLEM_CLOUD,
    "syncing": EMBLEM_SYNCING,
    "synced": EMBLEM_SYNCED,
    "cached": EMBLEM_SYNCED,
    "offline": EMBLEM_SYNCED,
    "error": EMBLEM_ERROR,
}


def _file_path_from_uri(uri: str) -> Optional[str]:
    """Convert a file:// URI to an absolute local path."""
    if uri.startswith("file://"):
        return urllib.parse.unquote(uri[7:])
    return None


def _get_client() -> Optional[IpcClient]:
    """Create an IPC client, returning None on failure."""
    try:
        return IpcClient(timeout=3.0)
    except Exception:
        return None


# ─── Emblem Provider ───────────────────────────────────────────────────────


class GDriverEmblemProvider(GObject.GObject, Nautilus.InfoProvider):
    """Adds sync-state emblem overlays to files managed by gDriver."""

    def update_file_info_full(self, provider, file_info):
        """Called by Nautilus for each visible file. Returns a completion status."""
        uri = file_info.get_uri()
        path = _file_path_from_uri(uri)
        if not path:
            return Nautilus.OperationComplete

        client = _get_client()
        if not client:
            return Nautilus.OperationComplete

        try:
            state = get_sync_state(client, path)
            if state and isinstance(state, dict):
                sync_state = state.get("state", "")
                emblem = EMBLEM_MAP.get(sync_state)
                if emblem:
                    file_info.add_emblem(emblem)
        finally:
            client.close()

        return Nautilus.OperationComplete


# ─── Menu Provider ─────────────────────────────────────────────────────────


class GDriverMenuProvider(GObject.GObject, Nautilus.MenuProvider):
    """Injects gDriver context menu items into Nautilus right-click menus."""

    def __init__(self):
        super().__init__()
        # Track current file path for menu callbacks.
        self._current_path: Optional[str] = None

    def get_file_items(self, *args) -> List[Nautilus.MenuItem]:
        """Return context menu items for the selected files."""
        # nautilus-python passes (provider, files) or just (files,) depending on version.
        files = args[-1]
        if not files or len(files) != 1:
            return []

        file_info = files[0]
        uri = file_info.get_uri()
        path = _file_path_from_uri(uri)
        if not path:
            return []

        # Only show menu for files under a Google Drive mount.
        if not self._is_drive_path(path):
            return []

        self._current_path = path

        items: List[Nautilus.MenuItem] = []

        # Submenu for all gDriver actions.
        submenu = Nautilus.Menu()
        root_item = Nautilus.MenuItem(
            name="GDriverMenu::root",
            label="gDrive",
            tip="Google Drive sync options",
        )
        root_item.set_submenu(submenu)

        # Check current state to decide which items to show.
        client = _get_client()
        current_state = None
        if client:
            try:
                state = get_sync_state(client, path)
                if state and isinstance(state, dict):
                    current_state = state.get("state", "")
            finally:
                client.close()

        # Available offline / Online only toggle.
        if current_state in ("cloud_only", "syncing"):
            item = Nautilus.MenuItem(
                name="GDriverMenu::available_offline",
                label="Make available offline",
                tip="Download this file and keep it available offline",
            )
            item.connect("activate", self._on_set_offline, path, True)
            submenu.append_item(item)
        elif current_state in ("synced", "cached", "offline"):
            item = Nautilus.MenuItem(
                name="GDriverMenu::online_only",
                label="Free up space",
                tip="Remove local copy, keep in cloud only",
            )
            item.connect("activate", self._on_set_offline, path, False)
            submenu.append_item(item)

        # Copy link.
        item = Nautilus.MenuItem(
            name="GDriverMenu::copy_link",
            label="Copy link",
            tip="Copy the Google Drive share link to clipboard",
        )
        item.connect("activate", self._on_copy_link, path)
        submenu.append_item(item)

        # View in Drive web.
        item = Nautilus.MenuItem(
            name="GDriverMenu::view_in_drive",
            label="View in Drive",
            tip="Open this file in Google Drive web",
        )
        item.connect("activate", self._on_view_in_drive, path)
        submenu.append_item(item)

        # Share.
        item = Nautilus.MenuItem(
            name="GDriverMenu::share",
            label="Share",
            tip="Share this file via Google Drive",
        )
        item.connect("activate", self._on_share, path)
        submenu.append_item(item)

        items.append(root_item)
        return items

    def _is_drive_path(self, path: str) -> bool:
        """Check if a path is under the Google Drive mount."""
        home = os.path.expanduser("~")
        drive_mount = os.path.join(home, "GoogleDrive")
        return path.startswith(drive_mount + "/") or path == drive_mount

    def _on_set_offline(
        self,
        _menu_item: Nautilus.MenuItem,
        path: str,
        enabled: bool,
    ):
        client = _get_client()
        if client:
            try:
                set_offline(client, path, enabled)
            finally:
                client.close()

    def _on_copy_link(self, _menu_item: Nautilus.MenuItem, path: str):
        client = _get_client()
        if not client:
            return
        try:
            url = get_share_link(client, path)
            if url:
                # Use xclip to copy to clipboard.
                try:
                    proc = subprocess.Popen(
                        ["xclip", "-selection", "clipboard"],
                        stdin=subprocess.PIPE,
                    )
                    proc.communicate(url.encode())
                except FileNotFoundError:
                    # Fallback: xdg-open the URL instead.
                    subprocess.Popen(["xdg-open", url])
        finally:
            client.close()

    def _on_view_in_drive(self, _menu_item: Nautilus.MenuItem, path: str):
        client = _get_client()
        if not client:
            return
        try:
            url = get_share_link(client, path)
            if url:
                subprocess.Popen(["xdg-open", url])
        finally:
            client.close()

    def _on_share(self, _menu_item: Nautilus.MenuItem, path: str):
        client = _get_client()
        if not client:
            return
        try:
            url = get_share_link(client, path)
            if url:
                # Open the Drive sharing dialog via web.
                share_url = url.replace("/view", "/edit#sharing")
                subprocess.Popen(["xdg-open", share_url])
        finally:
            client.close()
