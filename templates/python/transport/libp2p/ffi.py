"""
ffi.py — ctypes bindings to the borgkit-libp2p Rust shared library.

The library is loaded from (in order):
  1. BORGKIT_LIBP2P_LIB env var
  2. The directory containing this file (for bundled deployments)
  3. System library paths
"""
from __future__ import annotations

import ctypes
import ctypes.util
import os
import sys
from pathlib import Path
from typing import Callable, Optional

# ── locate the shared library ─────────────────────────────────────────────────

def _find_lib() -> str:
    # 1. Explicit env var override
    env = os.environ.get("BORGKIT_LIBP2P_LIB")
    if env:
        return env

    # 2. Alongside this file (bundled)
    here    = Path(__file__).parent
    suffix  = {"darwin": ".dylib", "win32": ".dll"}.get(sys.platform, ".so")
    bundled = here / f"libborgkit_libp2p{suffix}"
    if bundled.exists():
        return str(bundled)

    # 3. System lookup
    found = ctypes.util.find_library("borgkit_libp2p")
    if found:
        return found

    raise FileNotFoundError(
        "borgkit-libp2p shared library not found. "
        "Build it with: cd transport/rust && cargo build --release\n"
        "Then set BORGKIT_LIBP2P_LIB=/path/to/libborgkit_libp2p.so"
    )


# ── callback type ─────────────────────────────────────────────────────────────

RequestCallbackType = ctypes.CFUNCTYPE(ctypes.c_char_p, ctypes.c_char_p)


# ── BorgkitLibp2P ─────────────────────────────────────────────────────────────

class BorgkitLibp2P:
    """
    Low-level ctypes wrapper around the borgkit-libp2p C FFI.

    Usage
    -----
    node = BorgkitLibp2P()
    node.start("/ip4/0.0.0.0/tcp/0")
    print("peer_id:", node.peer_id())
    print("multiaddr:", node.multiaddr())

    # With request handler
    def handler(request_json: bytes) -> bytes:
        import json
        req = json.loads(request_json)
        return json.dumps({"requestId": req["requestId"], "status": "success",
                           "result": {"pong": True}, "timestamp": 0}).encode()

    node.start("/ip4/0.0.0.0/tcp/0", handler=handler)
    """

    _RESPONSE_BUF_SIZE = 1024 * 1024  # 1 MiB

    def __init__(self) -> None:
        self._lib    = ctypes.CDLL(_find_lib())
        self._handle: Optional[ctypes.c_void_p] = None
        self._cb_ref: Optional[RequestCallbackType] = None  # keep alive!
        self._setup_signatures()

    def _setup_signatures(self) -> None:
        lib = self._lib

        lib.borgkit_node_create.argtypes  = [ctypes.c_char_p, ctypes.c_void_p]
        lib.borgkit_node_create.restype   = ctypes.c_void_p

        lib.borgkit_node_destroy.argtypes = [ctypes.c_void_p]
        lib.borgkit_node_destroy.restype  = None

        lib.borgkit_node_peer_id.argtypes  = [ctypes.c_void_p]
        lib.borgkit_node_peer_id.restype   = ctypes.c_char_p

        lib.borgkit_node_multiaddr.argtypes = [ctypes.c_void_p]
        lib.borgkit_node_multiaddr.restype  = ctypes.c_char_p

        lib.borgkit_dial.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        lib.borgkit_dial.restype  = ctypes.c_int

        lib.borgkit_send.argtypes = [
            ctypes.c_void_p,   # handle
            ctypes.c_char_p,   # peer_id
            ctypes.c_char_p,   # request_json
            ctypes.c_char_p,   # response_buf
            ctypes.c_size_t,   # response_cap
        ]
        lib.borgkit_send.restype = ctypes.c_int

        lib.borgkit_gossip_publish.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        lib.borgkit_gossip_publish.restype  = ctypes.c_int

        lib.borgkit_free_string.argtypes = [ctypes.c_char_p]
        lib.borgkit_free_string.restype  = None

    def start(
        self,
        listen_addr:  str = "/ip4/0.0.0.0/tcp/0",
        handler:      Optional[Callable[[bytes], bytes]] = None,
    ) -> None:
        if self._handle is not None:
            return  # already started

        cb = None
        if handler is not None:
            def _wrapper(request_json: bytes) -> bytes:
                try:
                    return handler(request_json) or b'{}'
                except Exception as e:
                    import json
                    return json.dumps({"requestId": "", "status": "error",
                                       "errorMessage": str(e), "timestamp": 0}).encode()
            cb = RequestCallbackType(_wrapper)
            self._cb_ref = cb   # must keep reference alive

        self._handle = self._lib.borgkit_node_create(
            listen_addr.encode(),
            cb,
        )
        if not self._handle:
            raise RuntimeError("borgkit_node_create failed")

    def stop(self) -> None:
        if self._handle:
            self._lib.borgkit_node_destroy(self._handle)
            self._handle = None
            self._cb_ref = None

    def peer_id(self) -> str:
        self._require_started()
        raw = self._lib.borgkit_node_peer_id(self._handle)
        return raw.decode() if raw else ""

    def multiaddr(self) -> str:
        self._require_started()
        raw = self._lib.borgkit_node_multiaddr(self._handle)
        return raw.decode() if raw else ""

    def dial(self, multiaddr: str) -> None:
        self._require_started()
        rc = self._lib.borgkit_dial(self._handle, multiaddr.encode())
        if rc != 0:
            raise ConnectionError(f"borgkit_dial failed for {multiaddr}")

    def send(self, peer_id: str, request_json: str) -> str:
        self._require_started()
        buf = ctypes.create_string_buffer(self._RESPONSE_BUF_SIZE)
        n = self._lib.borgkit_send(
            self._handle,
            peer_id.encode(),
            request_json.encode(),
            buf,
            self._RESPONSE_BUF_SIZE,
        )
        if n < 0:
            raise RuntimeError(f"borgkit_send failed for peer {peer_id}")
        return buf.raw[:n].decode()

    def gossip_publish(self, message_json: str) -> None:
        self._require_started()
        rc = self._lib.borgkit_gossip_publish(self._handle, message_json.encode())
        if rc != 0:
            raise RuntimeError("borgkit_gossip_publish failed")

    def _require_started(self) -> None:
        if not self._handle:
            raise RuntimeError("BorgkitLibp2P not started — call .start() first")

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *_):
        self.stop()
