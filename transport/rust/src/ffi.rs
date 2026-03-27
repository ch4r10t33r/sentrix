//! C FFI — called by Python (ctypes) and Zig (@cImport).
//!
//! All functions are `extern "C"` and `#[no_mangle]`.
//! Strings are null-terminated UTF-8.
//! Ownership: strings returned by borgkit_* must be freed with borgkit_free_string().

use std::{
    ffi::{CStr, CString},
    os::raw::{c_char, c_int},
    ptr,
};

use crate::node::{AgentRequest, AgentResponse, BorgkitNode, BorgkitNodeConfig};

/// Opaque handle to a running BorgkitNode.
pub struct BorgkitHandle {
    node: BorgkitNode,
}

/// Callback type for incoming AgentRequests.
/// Called with a null-terminated JSON string, must return a null-terminated JSON string.
/// The returned string is freed by the Rust side after the callback returns.
pub type BorgkitRequestCallback =
    extern "C" fn(request_json: *const c_char) -> *mut c_char;

/// Create and start a new BorgkitNode.
///
/// `listen_addr` — multiaddr string, e.g. `/ip4/0.0.0.0/tcp/0`. NULL for default.
/// `handler`     — callback invoked for every incoming AgentRequest. NULL for no handler.
///
/// Returns an opaque handle, or NULL on error.
/// Free with `borgkit_node_destroy`.
#[no_mangle]
pub extern "C" fn borgkit_node_create(
    listen_addr: *const c_char,
    handler:     Option<BorgkitRequestCallback>,
) -> *mut BorgkitHandle {
    let addr_str = if listen_addr.is_null() {
        "/ip4/0.0.0.0/tcp/0".to_string()
    } else {
        unsafe { CStr::from_ptr(listen_addr) }.to_string_lossy().into_owned()
    };

    let config = BorgkitNodeConfig {
        listen_addrs: vec![addr_str.parse().unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".parse().unwrap())],
    };

    let cb: Option<Box<dyn Fn(AgentRequest) -> AgentResponse + Send + Sync>> =
        handler.map(|h| {
            let f: Box<dyn Fn(AgentRequest) -> AgentResponse + Send + Sync> =
                Box::new(move |req: AgentRequest| {
                    let json = match serde_json::to_string(&req) {
                        Ok(s)  => s,
                        Err(_) => return AgentResponse::error(&req.request_id, "serialisation error"),
                    };
                    let c_json = match CString::new(json) {
                        Ok(s)  => s,
                        Err(_) => return AgentResponse::error(&req.request_id, "null byte in json"),
                    };
                    let raw = h(c_json.as_ptr());
                    if raw.is_null() {
                        return AgentResponse::error(&req.request_id, "handler returned null");
                    }
                    let resp_str = unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned();
                    // The caller owns raw — Rust doesn't free it
                    serde_json::from_str(&resp_str).unwrap_or_else(|_| {
                        AgentResponse::error(&req.request_id, "invalid handler response")
                    })
                });
            f
        });

    match BorgkitNode::new(config, cb) {
        Ok(node) => Box::into_raw(Box::new(BorgkitHandle { node })),
        Err(_)   => ptr::null_mut(),
    }
}

/// Stop and destroy a BorgkitNode.
#[no_mangle]
pub extern "C" fn borgkit_node_destroy(handle: *mut BorgkitHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)); }
    }
}

/// Return the node's PeerId as a null-terminated string.
/// Caller must free with `borgkit_free_string`.
#[no_mangle]
pub extern "C" fn borgkit_node_peer_id(handle: *const BorgkitHandle) -> *mut c_char {
    if handle.is_null() { return ptr::null_mut(); }
    let s = unsafe { &*handle }.node.peer_id().to_string();
    CString::new(s).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

/// Return the node's first listen multiaddr as a null-terminated string.
/// Caller must free with `borgkit_free_string`.
#[no_mangle]
pub extern "C" fn borgkit_node_multiaddr(handle: *const BorgkitHandle) -> *mut c_char {
    if handle.is_null() { return ptr::null_mut(); }
    let addrs = unsafe { &*handle }.node.listen_addrs();
    let s = addrs.first().map(|a| a.to_string()).unwrap_or_default();
    CString::new(s).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

/// Dial a remote peer by multiaddr.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn borgkit_dial(
    handle: *mut BorgkitHandle,
    multiaddr: *const c_char,
) -> c_int {
    if handle.is_null() || multiaddr.is_null() { return -1; }
    let addr_str = unsafe { CStr::from_ptr(multiaddr) }.to_string_lossy();
    let addr = match addr_str.parse() {
        Ok(a)  => a,
        Err(_) => return -1,
    };
    match unsafe { &*handle }.node.dial(addr) {
        Ok(_)  => 0,
        Err(_) => -1,
    }
}

/// Send an AgentRequest JSON to a peer and write the AgentResponse JSON to `response_buf`.
///
/// `peer_id`      — target peer ID string
/// `request_json` — null-terminated JSON AgentRequest
/// `response_buf` — caller-allocated buffer for the JSON response
/// `response_cap` — capacity of `response_buf` in bytes
///
/// Returns bytes written (excluding null terminator), or -1 on error.
#[no_mangle]
pub extern "C" fn borgkit_send(
    handle:       *mut BorgkitHandle,
    peer_id:      *const c_char,
    request_json: *const c_char,
    response_buf: *mut c_char,
    response_cap: usize,
) -> c_int {
    if handle.is_null() || peer_id.is_null() || request_json.is_null() || response_buf.is_null() {
        return -1;
    }
    let pid_str  = unsafe { CStr::from_ptr(peer_id) }.to_string_lossy();
    let req_str  = unsafe { CStr::from_ptr(request_json) }.to_string_lossy();

    let peer: libp2p::PeerId = match pid_str.parse() {
        Ok(p)  => p,
        Err(_) => return -1,
    };
    let req: AgentRequest = match serde_json::from_str(&req_str) {
        Ok(r)  => r,
        Err(_) => return -1,
    };

    let resp = match unsafe { &*handle }.node.send(peer, req) {
        Ok(r)  => r,
        Err(_) => return -1,
    };
    let json = match serde_json::to_string(&resp) {
        Ok(s)  => s,
        Err(_) => return -1,
    };

    let bytes = json.as_bytes();
    if bytes.len() + 1 > response_cap { return -1; }

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), response_buf as *mut u8, bytes.len());
        *response_buf.add(bytes.len()) = 0;
    }
    bytes.len() as c_int
}

/// Publish a gossip message JSON to the mesh.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn borgkit_gossip_publish(
    handle:       *mut BorgkitHandle,
    message_json: *const c_char,
) -> c_int {
    if handle.is_null() || message_json.is_null() { return -1; }
    let json_str = unsafe { CStr::from_ptr(message_json) }.to_string_lossy();
    let data = json_str.as_bytes().to_vec();
    match unsafe { &*handle }.node.publish(data) {
        Ok(_)  => 0,
        Err(_) => -1,
    }
}

/// Free a string returned by a borgkit_* function.
#[no_mangle]
pub extern "C" fn borgkit_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)); }
    }
}
