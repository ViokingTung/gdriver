"""Synchronous IPC client for communicating with gdriver-daemon.

Implements JSON-RPC 2.0 over Unix Domain Socket (NDJSON framing),
matching the protocol used by the Rust `gdriver-ipc` crate.
"""

import json
import os
import socket
from typing import Any, Optional


def socket_path() -> str:
    """Return the path to the daemon IPC socket."""
    runtime = os.environ.get("XDG_RUNTIME_DIR", "")
    if runtime:
        return os.path.join(runtime, "gdriver.sock")
    return os.path.join("/tmp", "gdriver.sock")


class JsonRpcError(Exception):
    """JSON-RPC 2.0 error response."""

    def __init__(self, code: int, message: str, data: Any = None):
        super().__init__(message)
        self.code = code
        self.data = data


class IpcClient:
    """Synchronous blocking IPC client for communicating with gdriver-daemon.

    Opens a Unix Domain Socket, sends newline-delimited JSON-RPC 2.0
    requests, and reads back the corresponding response.  Push
    notifications from the daemon are silently discarded.
    """

    def __init__(self, timeout: float = 5.0):
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._sock.settimeout(timeout)
        self._sock.connect(socket_path())
        self._buf = b""
        self._next_id = 1

    def close(self):
        self._sock.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def call(self, method: str, params: Optional[dict] = None) -> Any:
        """Send a JSON-RPC request and wait for the matching response."""
        req_id = self._next_id
        self._next_id += 1

        request = {"jsonrpc": "2.0", "method": method, "id": req_id}
        if params is not None:
            request["params"] = params

        data = json.dumps(request) + "\n"
        self._sock.sendall(data.encode())

        while True:
            line = self._readline()
            if not line:
                raise JsonRpcError(-1, "daemon disconnected")

            try:
                resp = json.loads(line)
            except json.JSONDecodeError as e:
                raise JsonRpcError(-1, f"invalid JSON: {e}")

            # Push notifications have no id — skip them.
            if "id" not in resp:
                continue

            if "error" in resp:
                err = resp["error"]
                raise JsonRpcError(
                    err.get("code", -1),
                    err.get("message", "unknown error"),
                    err.get("data"),
                )

            return resp.get("result")

    def _readline(self) -> Optional[str]:
        """Read a newline-delimited line from the socket buffer."""
        while True:
            idx = self._buf.find(b"\n")
            if idx >= 0:
                line = self._buf[:idx].decode("utf-8", errors="replace")
                self._buf = self._buf[idx + 1 :]
                return line

            try:
                chunk = self._sock.recv(4096)
            except socket.timeout:
                raise JsonRpcError(-1, "socket timeout")
            if not chunk:
                return None
            self._buf += chunk


# ─── Convenience wrappers ───────────────────────────────────────────────────

METHOD_GET_SYNC_STATE = "fs.getSyncState"
METHOD_SET_OFFLINE = "fs.setOffline"
METHOD_GET_SHARE_LINK = "fs.getShareLink"


def get_sync_state(client: IpcClient, path: str) -> Optional[dict]:
    """Query the sync state for a file identified by its local path."""
    try:
        return client.call(METHOD_GET_SYNC_STATE, {"path": path})
    except JsonRpcError:
        return None


def set_offline(client: IpcClient, path: str, enabled: bool) -> bool:
    """Set a file's offline availability. Returns True on success."""
    try:
        client.call(METHOD_SET_OFFLINE, {"path": path, "enabled": enabled})
        return True
    except JsonRpcError:
        return False


def get_share_link(client: IpcClient, path: str) -> Optional[str]:
    """Get the Google Drive share link for a file."""
    try:
        result = client.call(METHOD_GET_SHARE_LINK, {"path": path})
        if isinstance(result, dict):
            return result.get("url")
        return None
    except JsonRpcError:
        return None
