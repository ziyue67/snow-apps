//! Take one screenshot of the desktop through the XDG desktop portal.
//!
//! This is the Wayland capture path end to end: the portal session is opened
//! (the compositor shows its picker the first time), the PipeWire frames are
//! read, and the result is written as a PNG next to the geometry that came with
//! it. It needs a real desktop session, so it is an example rather than a test:
//!
//! ```text
//! cargo run -p snow-capture --example portal_screenshot -- /tmp/portal.png
//! ```

use std::time::Duration;

use anyhow::{Context, Result};
use snow_capture::CaptureTarget;
use snow_capture::frame::CapturePixelFormat;
use snow_capture::system::{CaptureOptions, CaptureSystem};

fn main() -> Result<()> {
    let destination = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/portal-screenshot.png".to_string());

    let capabilities = snow_capture::capabilities::CaptureCapabilities::current();
    println!("backends: {:?}", capabilities.backends);
    println!("cpu formats: {:?}", capabilities.cpu_formats);

    let system = CaptureSystem::builder().build().context("system builds")?;
    let layout = system
        .monitor_layout()
        .context("the portal published a monitor layout")?;
    for monitor in &layout.monitors {
        println!(
            "monitor {} {} at {},{} {}x{} primary={}",
            monitor.monitor.stable_id(),
            monitor.monitor.name(),
            monitor.x,
            monitor.y,
            monitor.width,
            monitor.height,
            monitor.monitor.is_primary()
        );
    }
    println!(
        "virtual desktop {}x{} at {},{}",
        layout.virtual_width, layout.virtual_height, layout.virtual_left, layout.virtual_top
    );

    let options = CaptureOptions {
        output_pixel_format: CapturePixelFormat::Bgra8,
        ..CaptureOptions::default()
    };
    let mut session = system
        .open_session(CaptureTarget::PrimaryMonitor, options)
        .context("session opens")?;
    let started = std::time::Instant::now();
    let frame = session.capture_once().context("a frame arrives")?;
    println!(
        "captured {}x{} as {:?} in {} ms",
        frame.width(),
        frame.height(),
        frame.pixel_format(),
        started.elapsed().as_millis()
    );

    // Read the BGRA back into the byte order the encoder wants.
    let bytes = frame.as_bytes();
    let mut rgba = Vec::with_capacity(bytes.len());
    for pixel in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let image = image::RgbaImage::from_raw(frame.width(), frame.height(), rgba)
        .context("the frame has the expected size")?;
    image
        .save(&destination)
        .with_context(|| format!("the screenshot is written to {destination}"))?;
    println!("wrote {destination}");

    // The session is dropped here, which releases the cast and takes the
    // compositor's sharing indicator down with it.
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}
