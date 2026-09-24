//! Screen capture through the XDG desktop portal, via libportal.
//!
//! A Wayland session gives no way to read the composited desktop, so the portal
//! is asked for a screen cast and the frames arrive over PipeWire. libportal is
//! used because it is the library that exposes the pipewire remote call.

#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
struct XdpPortal {
    _private : [u8; 0],
}

#[repr(C)]
struct XdpSession {
    _private : [u8; 0],
}

#[repr(C)]
struct GError {
    _private : [u8; 0],
}

type GAsyncReadyCallback = Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void)>;

unsafe extern "C" {
    fn xdp_portal_new() -> * mut XdpPortal;
    fn xdp_portal_create_screencast_session(
        portal : *mut XdpPortal, outputs : c_int, flags : c_int, cursor_mode : c_int,
        persist_mode : c_int, restore_token : * const c_char, cancellable : *mut c_void,
        callback : GAsyncReadyCallback, data : *mut c_void, );
    fn xdp_portal_create_screencast_session_finish(portal : *mut XdpPortal, result : *mut c_void,
                                                   error : *mut * mut GError, ) -> * mut XdpSession;
    fn xdp_session_start(session : *mut XdpSession, parent : *mut c_void, cancellable : *mut c_void,
                         callback : GAsyncReadyCallback, data : *mut c_void, );
    fn xdp_session_start_finish(session : *mut XdpSession, result : *mut c_void,
                                error : *mut * mut GError, ) -> c_int;
    fn xdp_session_open_pipewire_remote(session : *mut XdpSession) -> c_int;
    fn g_main_context_new() -> * mut c_void;
    fn g_main_context_unref(context : *mut c_void);
    fn g_main_context_push_thread_default(context : *mut c_void);
    fn g_main_context_pop_thread_default(context : *mut c_void);
    fn g_main_context_iteration(context : *mut c_void, may_block : c_int) -> c_int;
}

/// The portal output type for monitors.
const XDP_OUTPUT_MONITOR : c_int = 1;
/// `XDP_CURSOR_MODE_EMBEDDED`: the compositor draws the cursor into the frame.
const XDP_CURSOR_MODE_EMBEDDED : c_int = 2;
/// `XDP_PERSIST_MODE_NONE`: nothing about the session is remembered.
const XDP_PERSIST_MODE_NONE : c_int = 0;
/// How long to keep asking the portal before giving up. The user has to answer a
/// permission prompt, so this is generous.
const PORTAL_TIMEOUT_MS : u128 = 120_000;

/// Where the portal writes the reply it posts.
struct CreateState {
    result : *mut c_void,
}

unsafe extern "C" fn
on_created(_source : *mut c_void, result : *mut c_void, data : *mut c_void) {
    unsafe {
        let state = data.cast::<CreateState>();
        (*state).result = result;
    }
}

unsafe extern "C" fn on_started(_source : *mut c_void, result : *mut c_void, data : *mut c_void) {
    unsafe {
        let state = data.cast::<CreateState>();
        (*state).result = result;
    }
}

/// Drain the context until the portal answers.
///
/// A main loop is not used on purpose: it returns as soon as the context has
/// nothing attached, which happened before the portal posted its reply and left
/// the result null, and finishing with a null result is a crash.
fn wait_for_result(context : *mut c_void, state : &CreateState) -> bool {
    let started = std::time::Instant::now();
    while
        state.result.is_null() {
            while
                unsafe{g_main_context_iteration(context, 0)} != 0 {}
            if started
                .elapsed().as_millis() > PORTAL_TIMEOUT_MS {
                    return false;
                }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    true
}

/// Open a screen cast session and hand back the pipewire remote fd.
pub(crate) fn open_portal_stream()->anyhow::Result<c_int> {
    unsafe {
        let portal = xdp_portal_new();
        if portal
            .is_null() {
                anyhow::bail !("could not create a portal handle");
            }

        // The portal posts its reply on the thread default context, so make this
        // call's own context that one for the duration.
        let context = g_main_context_new();
        g_main_context_push_thread_default(context);

        let mut state = CreateState{
            result : std::ptr::null_mut(),
        };
        xdp_portal_create_screencast_session(
            portal, XDP_OUTPUT_MONITOR, 0, XDP_CURSOR_MODE_EMBEDDED, XDP_PERSIST_MODE_NONE,
            std::ptr::null(), std::ptr::null_mut(), Some(on_created),
            std::ptr::from_mut(&mut state).cast(), );
        if !wait_for_result (context, &state) {
            g_main_context_pop_thread_default(context);
            g_main_context_unref(context);
            anyhow::bail !("the portal did not answer in time");
        }

        let mut error : * mut GError = std::ptr::null_mut();
        let session = xdp_portal_create_screencast_session_finish(portal, state.result, &mut error);
        if session
            .is_null() {
                g_main_context_pop_thread_default(context);
                g_main_context_unref(context);
                anyhow::bail !("the portal refused to create a screen cast session");
            }

        state.result = std::ptr::null_mut();
        xdp_session_start(session, std::ptr::null_mut(), std::ptr::null_mut(), Some(on_started),
                          std::ptr::from_mut(&mut state).cast(), );
        if !wait_for_result (context, &state) {
            g_main_context_pop_thread_default(context);
            g_main_context_unref(context);
            anyhow::bail !("the portal did not answer in time");
        }

        if xdp_session_start_finish (session, state.result, &mut error)
            == 0 {
                g_main_context_pop_thread_default(context);
                g_main_context_unref(context);
                anyhow::bail !("the portal refused to start the screen cast session");
            }

        let fd = xdp_session_open_pipewire_remote(session);
        g_main_context_pop_thread_default(context);
        g_main_context_unref(context);
        if fd
            < 0 {
                anyhow::bail !("the portal returned no pipewire remote");
            }
        Ok(fd)
    }
}
