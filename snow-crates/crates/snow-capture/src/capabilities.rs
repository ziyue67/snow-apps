//! Feature discovery is separate from permission requests and target acquisition.
use crate::backend::CaptureBackendKind;
use snow_media::{PixelFormat, geometry::DesktopSpace};
#[derive(Clone, Debug)]
pub struct CaptureCapabilities {
    pub backends: Vec<CaptureBackendKind>,
    pub desktop_space: DesktopSpace,
    pub cpu_formats: Vec<PixelFormat>,
    pub native_frames: bool,
    pub hdr_capture: bool,
    pub window_enumeration: bool,
}
impl CaptureCapabilities {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            let native = snow_macos::capabilities::CaptureSupport::current();
            let supported = native.screen_capture_kit && native.metal_composition;
            let hdr = supported && native.hdr_capture;
            let mut formats = vec![PixelFormat::Rgba8, PixelFormat::Bgra8];
            if hdr {
                formats.push(PixelFormat::Rgba16Float);
            }
            Self {
                backends: if supported {
                    vec![CaptureBackendKind::ScreenCaptureKit]
                } else {
                    vec![]
                },
                desktop_space: DesktopSpace::Points,
                cpu_formats: if supported { formats } else { vec![] },
                native_frames: supported,
                hdr_capture: hdr,
                window_enumeration: supported,
            }
        }
        #[cfg(target_os = "windows")]
        {
            Self {
                backends: vec![
                    CaptureBackendKind::DxgiDuplication,
                    CaptureBackendKind::WindowsGraphicsCapture,
                    CaptureBackendKind::Gdi,
                ],
                desktop_space: DesktopSpace::PhysicalPixels,
                cpu_formats: vec![PixelFormat::Rgba8, PixelFormat::Bgra8],
                native_frames: true,
                hdr_capture: false,
                window_enumeration: false,
            }
        }
        // X11 capture reads the root window into CPU memory, so it advertises no
        // native frames; window capture and per-output enumeration are not
        // implemented yet, which is why window_enumeration stays false.
        #[cfg(target_os = "linux")]
        {
            // Under Wayland the X root window belongs to XWayland and has no
            // screen content, so the desktop is read through the portal's
            // screen cast: the compositor publishes each selected monitor over
            // PipeWire and the frames arrive as CPU-visible BGRA.
            if crate::platform::linux::wayland_session() {
                return Self {
                    backends: vec![CaptureBackendKind::Portal],
                    desktop_space: DesktopSpace::PhysicalPixels,
                    cpu_formats: vec![PixelFormat::Bgra8],
                    native_frames: false,
                    hdr_capture: false,
                    window_enumeration: false,
                };
            }
            Self {
                backends: vec![CaptureBackendKind::X11],
                desktop_space: DesktopSpace::PhysicalPixels,
                cpu_formats: vec![PixelFormat::Bgra8],
                native_frames: false,
                hdr_capture: false,
                window_enumeration: false,
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Self {
                backends: vec![],
                desktop_space: DesktopSpace::PhysicalPixels,
                cpu_formats: vec![],
                native_frames: false,
                hdr_capture: false,
                window_enumeration: false,
            }
        }
    }
}
