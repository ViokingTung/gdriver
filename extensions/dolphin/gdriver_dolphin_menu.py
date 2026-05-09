#!/usr/bin/env python3
"""Dolphin service menu action handler for gDriver.

This script is invoked by the Dolphin service menu .desktop file.
It receives the selected file path and action name as arguments,
then communicates with gdriver-daemon via IPC.

Usage:
    gdriver_dolphin_menu.py <action> <file_path>

Actions:
    available_offline  - Make file available offline
    online_only        - Free up space (remove local copy)
    copy_link          - Copy Drive share link to clipboard
    view_in_drive      - Open file in Google Drive web
    share              - Open Drive sharing dialog
"""

import subprocess
import sys
import urllib.parse

from gdriver_dolphin_ipc import IpcClient, get_share_link, set_offline


def main():
    if len(sys.argv) < 3:
        print("Usage: gdriver_dolphin_menu.py <action> <file_path>", file=sys.stderr)
        sys.exit(1)

    action = sys.argv[1]
    # Dolphin may pass file:// URIs or plain paths.
    raw_path = sys.argv[2]
    if raw_path.startswith("file://"):
        path = urllib.parse.unquote(raw_path[7:])
    else:
        path = raw_path

    try:
        client = IpcClient(timeout=5.0)
    except Exception as e:
        print(f"Failed to connect to gdriver-daemon: {e}", file=sys.stderr)
        sys.exit(1)

    try:
        if action == "available_offline":
            set_offline(client, path, True)
        elif action == "online_only":
            set_offline(client, path, False)
        elif action == "copy_link":
            url = get_share_link(client, path)
            if url:
                _copy_to_clipboard(url)
        elif action == "view_in_drive":
            url = get_share_link(client, path)
            if url:
                subprocess.Popen(["xdg-open", url])
        elif action == "share":
            url = get_share_link(client, path)
            if url:
                share_url = url.replace("/view", "/edit#sharing")
                subprocess.Popen(["xdg-open", share_url])
        else:
            print(f"Unknown action: {action}", file=sys.stderr)
            sys.exit(1)
    finally:
        client.close()


def _copy_to_clipboard(text: str):
    """Copy text to clipboard using available clipboard tool."""
    # Try xclip first, then xsel, then wl-copy (Wayland).
    for cmd in [
        ["xclip", "-selection", "clipboard"],
        ["xsel", "--clipboard", "--input"],
        ["wl-copy"],
    ]:
        try:
            proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
            proc.communicate(text.encode())
            if proc.returncode == 0:
                return
        except FileNotFoundError:
            continue

    # Fallback: use Qt's QClipboard via qdbus if available.
    try:
        subprocess.run(
            ["qdbus", "org.kde.klipper", "/klipper", "setClipboardContents", text],
            check=True,
            capture_output=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        print("Warning: No clipboard tool available", file=sys.stderr)


if __name__ == "__main__":
    main()
