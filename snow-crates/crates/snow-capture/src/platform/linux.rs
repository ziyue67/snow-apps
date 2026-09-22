//! X11 screen capture backend.
//!
//! Binds Xlib directly instead of adding a dependency: the packaging already
//! requires `libX11`, and only a handful of calls are needed. The captured
//! buffer is handed to [`Frame::from_bgra8`] because a 32-bit TrueColor
//! `ZPixmap` stores each pixel as `0x00RRGGBB`, which on a little-endian host
//! reads back as BGRA.
//!
//! Multi-head setups are reported as a single monitor spanning the X screen:
//! enough for whole-screen capture, which is what a screenshot needs first.
//! Per-output enumeration through RandR and window capture through XComposite
//! are follow-up work.

use std::os::raw::{c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;
use std::sync::Once;

use crate::backend::{CaptureBackend, CaptureBackendKind, MonitorCapturer};
use crate::capture_session::CaptureTargetInfo;
use crate::error::{CaptureError, CaptureResult};
use crate::frame::Frame;
use crate::monitor::MonitorId;
use crate::region::{MonitorGeometry, MonitorLayout};
use crate::window::WindowId;

/// `ZPixmap` from `X.h`.
const X_Z_PIXMAP: c_int = 2;
/// `AllPlanes` from `X.h`.
const X_ALL_PLANES: c_ulong = !0;
/// `XImage` byte order for a little-endian server, matching this host.
const X_LSB_FIRST: c_int = 0;

// Only the leading fields of `XImage` are declared; the layout up to
// `blue_mask` is fixed by Xlib.h, and nothing past it is read.
#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XInitThreads() -> c_int;
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(display: *mut c_void) -> c_int;
    fn XDefaultScreen(display: *mut c_void) -> c_int;
    fn XDisplayWidth(display: *mut c_void, screen: c_int) -> c_int;
    fn XDisplayHeight(display: *mut c_void, screen: c_int) -> c_int;
    fn XRootWindow(display: *mut c_void, screen: c_int) -> c_ulong;
    fn XGetImage(
        display: *mut c_void,
        drawable: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        plane_mask: c_ulong,
        format: c_int,
    ) -> *mut XImage;
    fn XDestroyImage(image: *mut XImage) -> c_int;
}

fn unsupported(what: &str) -> CaptureError {
    CaptureError::platform(anyhow::anyhow!(
        "{what} is not implemented by the X11 capture backend"
    ))
}

/// A display connection closed on drop, so every call site can use `?`.
struct Display(*mut c_void);

impl Display {
    fn open() -> CaptureResult<Self> {
        static INIT: Once = Once::new();
        // Xlib is not thread-safe unless initialised before the first
        // connection; capture runs on worker threads, so do it once up front.
        INIT.call_once(|| unsafe {
            XInitThreads();
        });
        let display = unsafe { XOpenDisplay(ptr::null()) };
        if display.is_null() {
            return Err(CaptureError::platform(anyhow::anyhow!(
                "could not open an X display; is DISPLAY set?"
            )));
        }
        Ok(Self(display))
    }

    fn screen(&self) -> c_int {
        unsafe { XDefaultScreen(self.0) }
    }

    fn root_window(&self) -> c_ulong {
        unsafe { XRootWindow(self.0, self.screen()) }
    }

    fn screen_size(&self) -> (c_int, c_int) {
        let screen = self.screen();
        unsafe {
            (
                XDisplayWidth(self.0, screen),
                XDisplayHeight(self.0, screen),
            )
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        unsafe {
            XCloseDisplay(self.0);
        }
    }
}

fn screen_monitor(display: &Display) -> MonitorId {
    MonitorId::from_name(i64::from(display.screen()) as isize, "X11 screen", true)
}

/// Read the pixels of the X screen into a frame.
fn capture_screen() -> CaptureResult<Frame> {
    let display = Display::open()?;
    let (width, height) = display.screen_size();
    if width <= 0 || height <= 0 {
        return Err(CaptureError::platform(anyhow::anyhow!(
            "the X screen reports an empty geometry ({width}x{height})"
        )));
    }

    let image = unsafe {
        XGetImage(
            display.0,
            display.root_window(),
            0,
            0,
            width as c_uint,
            height as c_uint,
            X_ALL_PLANES,
            X_Z_PIXMAP,
        )
    };
    if image.is_null() {
        return Err(CaptureError::platform(anyhow::anyhow!(
            "XGetImage returned no image for the root window"
        )));
    }

    // Copy out of the X server's buffer before releasing it.
    let buffer = unsafe {
        let image = &*image;
        let result = if image.bits_per_pixel == 32
            && image.byte_order == X_LSB_FIRST
            && !image.data.is_null()
        {
            let stride = image.bytes_per_line as usize;
            let row_bytes = (image.width as usize) * 4;
            let mut pixels = Vec::with_capacity(row_bytes * image.height as usize);
            for row in 0..image.height as usize {
                let start = image.data.add(row * stride) as *const u8;
                pixels.extend_from_slice(std::slice::from_raw_parts(start, row_bytes));
            }
            Ok(pixels)
        } else {
            Err(CaptureError::platform(anyhow::anyhow!(
                "unsupported X image layout: {} bits per pixel, byte order {}",
                image.bits_per_pixel,
                image.byte_order
            )))
        };
        XDestroyImage(image as *const XImage as *mut XImage);
        result
    }?;

    Frame::from_bgra8(width as u32, height as u32, buffer)
}

/// Geometry of the X screen as a single-monitor layout.
pub(crate) fn layout() -> CaptureResult<MonitorLayout> {
    let display = Display::open()?;
    let (width, height) = display.screen_size();
    if width <= 0 || height <= 0 {
        return Err(CaptureError::platform(anyhow::anyhow!(
            "the X screen reports an empty geometry ({width}x{height})"
        )));
    }
    let monitor = screen_monitor(&display);
    Ok(MonitorLayout {
        monitors: vec![MonitorGeometry {
            monitor,
            x: 0,
            y: 0,
            width: width as u32,
            height: height as u32,
        }],
        virtual_left: 0,
        virtual_top: 0,
        virtual_width: width as u32,
        virtual_height: height as u32,
    })
}

/// The X11 capture backend. It enumerates one monitor covering the X screen.
pub(crate) struct X11Backend;

impl CaptureBackend for X11Backend {
    fn enumerate_monitors(&self) -> CaptureResult<Vec<MonitorId>> {
        let display = Display::open()?;
        Ok(vec![screen_monitor(&display)])
    }

    fn primary_monitor(&self) -> CaptureResult<MonitorId> {
        let display = Display::open()?;
        Ok(screen_monitor(&display))
    }

    fn monitor_layout(&self) -> CaptureResult<MonitorLayout> {
        layout()
    }

    fn inspect_window(&self, _window: &WindowId) -> CaptureResult<CaptureTargetInfo> {
        Err(unsupported("window inspection"))
    }

    fn create_monitor_capturer(
        &self,
        _monitor: &MonitorId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        Ok(Box::new(X11MonitorCapturer))
    }

    fn create_window_capturer(
        &self,
        _window: &WindowId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        Err(unsupported("window capture"))
    }
}

struct X11MonitorCapturer;

impl MonitorCapturer for X11MonitorCapturer {
    fn backend_kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::X11
    }

    fn capture(&mut self, _reuse: Option<Frame>) -> CaptureResult<Frame> {
        capture_screen()
    }
}
