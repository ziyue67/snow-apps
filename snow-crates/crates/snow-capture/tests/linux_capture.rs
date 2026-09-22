//! X11 capture backend checks.
//!
//! These need an X display; they are skipped when `DISPLAY` is unset, so an
//! ordinary headless `cargo test` run stays green. Point them at a virtual
//! display with known dimensions to assert real pixels:
//!
//! ```text
//! xvfb-run -s "-screen 0 320x240x24" cargo test -p snow-capture --test linux_capture
//! ```
#![cfg(target_os = "linux")]

use snow_capture::backend::CaptureBackendKind;
use snow_capture::capabilities::CaptureCapabilities;
use snow_capture::system::{CaptureOptions, CaptureSystem};
use snow_capture::CaptureTarget;

/// Capability discovery must not depend on a display: report the backend the
/// platform actually provides even when no connection can be opened.
#[test]
fn advertises_the_x11_backend() {
    let capabilities = CaptureCapabilities::current();
    assert_eq!(
        capabilities.backends,
        vec![CaptureBackendKind::X11],
        "Linux advertises the X11 backend"
    );
    assert!(
        !capabilities.cpu_formats.is_empty(),
        "the backend produces CPU frames, so it advertises formats"
    );
    assert!(
        !capabilities.native_frames,
        "X11 capture reads into CPU memory rather than handing over native frames"
    );
    assert!(
        !capabilities.window_enumeration,
        "window enumeration is not implemented yet"
    );
}

fn display_available() -> bool {
    std::env::var_os("DISPLAY").is_some()
}

#[test]
fn enumerates_the_x_screen_as_one_monitor() {
    if !display_available() {
        eprintln!("skipping: DISPLAY is not set");
        return;
    }
    let system = CaptureSystem::builder().build().expect("system builds");
    let monitors = system.enumerate_monitors().expect("monitors enumerate");
    assert_eq!(monitors.len(), 1, "the X screen is reported as one monitor");

    let primary = system.primary_monitor().expect("primary monitor resolves");
    assert_eq!(primary, monitors[0], "the screen is the primary monitor");
}

#[test]
fn reports_a_layout_with_a_positive_extent() {
    if !display_available() {
        eprintln!("skipping: DISPLAY is not set");
        return;
    }
    let system = CaptureSystem::builder().build().expect("system builds");
    let layout = system.monitor_layout().expect("layout resolves");
    assert_eq!(layout.monitors.len(), 1);
    assert!(layout.virtual_width > 0, "virtual width is positive");
    assert!(layout.virtual_height > 0, "virtual height is positive");

    let geometry = &layout.monitors[0];
    assert_eq!(geometry.width, layout.virtual_width);
    assert_eq!(geometry.height, layout.virtual_height);
}

#[test]
fn captures_a_frame_matching_the_screen_geometry() {
    if !display_available() {
        eprintln!("skipping: DISPLAY is not set");
        return;
    }
    let system = CaptureSystem::builder().build().expect("system builds");
    let layout = system.monitor_layout().expect("layout resolves");
    assert_eq!(layout.virtual_width, 320, "virtual width matches the screen");
    assert_eq!(layout.virtual_height, 240, "virtual height matches the screen");

    let mut session = system
        .open_session(CaptureTarget::PrimaryMonitor, CaptureOptions::default())
        .expect("session opens");
    let frame = session.capture_once().expect("capture succeeds");
    assert_eq!(frame.width(), layout.virtual_width);
    assert_eq!(frame.height(), layout.virtual_height);
}
