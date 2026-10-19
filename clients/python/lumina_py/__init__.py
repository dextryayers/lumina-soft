"""Standard-library client for the Lumina IPC protocol.

Example:

    from lumina_py import LuminaClient

    with LuminaClient() as client:
        print(client.status()["state"])
        client.notify("Build done", body="lumina 0.1.0-alpha")
"""

from .client import (
    AsyncLuminaClient,
    ConnectionClosed,
    LuminaClient,
    LuminaError,
    ProtocolError,
    RemoteError,
)
from .transport import MAX_LINE_BYTES, FrameError, default_address

__all__ = [
    "AsyncLuminaClient",
    "ConnectionClosed",
    "FrameError",
    "LuminaClient",
    "LuminaError",
    "MAX_LINE_BYTES",
    "ProtocolError",
    "RemoteError",
    "default_address",
]

__version__ = "0.1.0a1"
