//! PipeWire consumer for the streams a portal screen cast session publishes.
//!
//! The portal hands back a file descriptor for its PipeWire remote rather than
//! pixels, so the frames have to be taken off the graph: this module runs a
//! PipeWire main loop on its own thread, connects one input stream per portal
//! stream, and keeps the newest frame of each one for the caller to pick up.
//!
//! Frames arrive as raw video, so the negotiable formats are restricted to the
//! 8-bit RGB layout (the compositor picks one of them) and every frame is
//! converted to the BGRA the rest of the capture layer works in.

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pipewire as pw;
use pw::spa;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::Pod;
use pw::spa::utils::Direction;

use crate::error::{CaptureError, CaptureResult};
use crate::frame::Frame;

/// Whether the stream should describe what it negotiates and receives.
///
/// A Wayland capture can fail in several places that all look alike from the
/// outside (no stream, no format, no buffer, no frame), so the negotiation and
/// the frame counters can be printed with `SNOW_CAPTURE_PORTAL_DEBUG=1`.
fn debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SNOW_CAPTURE_PORTAL_DEBUG").is_some_and(|value| !value.is_empty())
    })
}

macro_rules! portal_debug {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!("[snow-portal] {}", format_args!($($arg)*));
        }
    };
}

/// How long the first frame may take once the streams are connected. The
/// compositor starts streaming as soon as the session is started, so this is
/// only a guard against a graph that never delivers.
pub(crate) const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// One stream the portal published for the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StreamSource {
    /// PipeWire node the compositor exports for this stream.
    pub node_id: u32,
    /// Stable `pipewire-serial` of that node, when the portal named one.
    pub target_object: Option<String>,
    /// Position of the stream on the desktop, in physical pixels, when known.
    pub position: Option<(i32, i32)>,
    /// Size of the stream, in physical pixels, when the portal named one.
    pub size: Option<(u32, u32)>,
}

/// A frame taken from one portal stream, with the stream it came from.
pub(crate) struct StreamFrame {
    #[allow(dead_code)]
    pub source: StreamSource,
    pub frame: Frame,
}

/// What one stream has produced so far.
struct Slot {
    source: StreamSource,
    frame: Option<Frame>,
    error: Option<String>,
    /// Last stream state, kept for the timeout message.
    state: &'static str,
}

struct Shared {
    slots: Mutex<Vec<Slot>>,
    ready: Condvar,
    /// Set when the loop itself could not reach the graph at all.
    session_error: Mutex<Option<String>>,
    stop: AtomicBool,
}

impl Shared {
    fn take_session_error(&self) -> Option<String> {
        self.session_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
    }

    fn report_session_error(&self, message: String) {
        if let Ok(mut error) = self.session_error.lock()
            && error.is_none()
        {
            *error = Some(message);
        }
        self.ready.notify_all();
    }
}

/// The consumers for every stream of one portal session.
pub(crate) struct PortalStreams {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PortalStreams {
    /// Connect to the PipeWire remote the portal returned and start reading.
    ///
    /// The thread owns the loop, so this returns as soon as the work is queued:
    /// [`PortalStreams::capture`] waits for the actual frames.
    pub(crate) fn start(fd: OwnedFd, sources: Vec<StreamSource>) -> CaptureResult<Self> {
        if sources.is_empty() {
            return Err(CaptureError::platform(anyhow::anyhow!(
                "the portal session published no screen cast stream"
            )));
        }

        let shared = Arc::new(Shared {
            slots: Mutex::new(
                sources
                    .into_iter()
                    .map(|source| Slot {
                        source,
                        frame: None,
                        error: None,
                        state: "unconnected",
                    })
                    .collect(),
            ),
            ready: Condvar::new(),
            session_error: Mutex::new(None),
            stop: AtomicBool::new(false),
        });

        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("snow-portal-pipewire".to_string())
            .spawn(move || run(fd, thread_shared))
            .map_err(|error| {
                CaptureError::platform(anyhow::anyhow!(
                    "could not start the PipeWire thread: {error}"
                ))
            })?;

        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Wait for a frame from every stream and return them with their geometry.
    pub(crate) fn capture(&self, timeout: Duration) -> CaptureResult<Vec<StreamFrame>> {
        let deadline = Instant::now() + timeout;
        let mut slots = self.shared.slots.lock().map_err(|_| {
            CaptureError::platform(anyhow::anyhow!("the capture state was poisoned"))
        })?;

        loop {
            if let Some(error) = self.shared.take_session_error() {
                return Err(CaptureError::platform(anyhow::anyhow!(error)));
            }
            if let Some(error) = slots.iter().find_map(|slot| slot.error.clone()) {
                return Err(CaptureError::platform(anyhow::anyhow!(error)));
            }
            if slots.iter().all(|slot| slot.frame.is_some()) {
                break;
            }

            let now = Instant::now();
            if now >= deadline {
                let detail = slots
                    .iter()
                    .map(|slot| format!("node {} is {}", slot.source.node_id, slot.state))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(CaptureError::platform(anyhow::anyhow!(
                    "the screen cast produced no frame within {} ms ({detail})",
                    timeout.as_millis()
                )));
            }

            let (guard, _) = self
                .shared
                .ready
                .wait_timeout(slots, deadline - now)
                .map_err(|_| {
                    CaptureError::platform(anyhow::anyhow!("the capture state was poisoned"))
                })?;
            slots = guard;
        }

        let frames = slots
            .iter()
            .filter_map(|slot| {
                slot.frame.clone().map(|frame| StreamFrame {
                    source: slot.source.clone(),
                    frame,
                })
            })
            .collect();
        Ok(frames)
    }
}

impl Drop for PortalStreams {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // The loop checks the flag on a timer, so joining is bounded by one
            // timer tick; a thread that already died is joined just as well.
            let _ = thread.join();
        }
    }
}

/// Read every byte of a frame row into a fresh BGRA buffer.
fn copy_rows_to_bgra(
    source: &[u8],
    offset: usize,
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
) -> CaptureResult<Vec<u8>> {
    let layout = FormatLayout::of(format).ok_or_else(|| {
        CaptureError::platform(anyhow::anyhow!(
            "the compositor negotiated the unsupported video format {format:?}"
        ))
    })?;

    // The source row is as wide as the negotiated format, while the frame this
    // produces is always BGRA.
    let source_row_bytes = (width as usize) * layout.bytes_per_pixel;
    let output_row_bytes = (width as usize) * 4;
    let stride = if stride == 0 {
        source_row_bytes
    } else {
        stride
    };
    let end = offset
        .checked_add(stride.saturating_mul(height as usize))
        .ok_or_else(|| CaptureError::platform(anyhow::anyhow!("the frame geometry overflowed")))?;
    if end > source.len() {
        return Err(CaptureError::platform(anyhow::anyhow!(
            "the compositor delivered a {width}x{height} {format:?} frame of {} bytes with an \
             offset of {offset} and a stride of {stride}",
            source.len()
        )));
    }

    let mut pixels = vec![0u8; output_row_bytes * height as usize];
    for row in 0..height as usize {
        let start = offset + row * stride;
        let src = &source[start..start + source_row_bytes];
        let dst = &mut pixels[row * output_row_bytes..(row + 1) * output_row_bytes];
        layout.write_bgra_row(src, dst);
    }
    Ok(pixels)
}

/// The byte layout of one negotiated raw video format.
#[derive(Clone, Copy)]
struct FormatLayout {
    bytes_per_pixel: usize,
    /// Byte index of blue, green, red and alpha inside one pixel, with
    /// `usize::MAX` for a channel the format does not carry (opaque output).
    channels: [usize; 4],
}

impl FormatLayout {
    fn of(format: VideoFormat) -> Option<Self> {
        // SPA names formats by their byte order in memory: `BGRA` is
        // B, G, R, A. Anything the compositor may negotiate for a screen cast
        // is four bytes wide, and the three-byte legacy formats are expanded.
        let four = |channels: [usize; 4]| Self {
            bytes_per_pixel: 4,
            channels,
        };
        match format {
            VideoFormat::BGRA => Some(four([0, 1, 2, 3])),
            VideoFormat::BGRx => Some(four([0, 1, 2, usize::MAX])),
            VideoFormat::RGBA => Some(four([2, 1, 0, 3])),
            VideoFormat::RGBx => Some(four([2, 1, 0, usize::MAX])),
            VideoFormat::ARGB => Some(four([3, 2, 1, 0])),
            VideoFormat::ABGR => Some(four([1, 2, 3, 0])),
            VideoFormat::xRGB => Some(four([3, 2, 1, usize::MAX])),
            VideoFormat::xBGR => Some(four([1, 2, 3, usize::MAX])),
            VideoFormat::RGB => Some(Self {
                bytes_per_pixel: 3,
                channels: [2, 1, 0, usize::MAX],
            }),
            VideoFormat::BGR => Some(Self {
                bytes_per_pixel: 3,
                channels: [0, 1, 2, usize::MAX],
            }),
            _ => None,
        }
    }

    fn write_bgra_row(&self, src: &[u8], dst: &mut [u8]) {
        for (pixel, out) in src
            .chunks_exact(self.bytes_per_pixel)
            .zip(dst.chunks_exact_mut(4))
        {
            for (channel, index) in self.channels.iter().enumerate() {
                out[channel] = if *index == usize::MAX {
                    255
                } else {
                    pixel[*index]
                };
            }
        }
    }
}

/// The formats offered to the compositor, most preferred first.
const OFFERED_FORMATS: [VideoFormat; 4] = [
    VideoFormat::BGRx,
    VideoFormat::BGRA,
    VideoFormat::xRGB,
    VideoFormat::RGBx,
];

/// State the stream callbacks share with the loop.
struct StreamState {
    index: usize,
    shared: Arc<Shared>,
    format: VideoInfoRaw,
    /// Frames seen so far, so the diagnostics can stay quiet between them.
    frames: u64,
}

impl StreamState {
    fn store_frame(&self, frame: Frame) {
        if let Ok(mut slots) = self.shared.slots.lock()
            && let Some(slot) = slots.get_mut(self.index)
        {
            slot.frame = Some(frame);
            slot.error = None;
        }
        self.shared.ready.notify_all();
    }

    fn store_error(&self, message: String) {
        if let Ok(mut slots) = self.shared.slots.lock()
            && let Some(slot) = slots.get_mut(self.index)
            && slot.frame.is_none()
        {
            slot.error = Some(message);
        }
        self.shared.ready.notify_all();
    }

    fn set_state(&self, state: &'static str) {
        if let Ok(mut slots) = self.shared.slots.lock()
            && let Some(slot) = slots.get_mut(self.index)
        {
            slot.state = state;
        }
    }
}

fn stream_state_name(state: &pw::stream::StreamState) -> &'static str {
    match state {
        pw::stream::StreamState::Error(_) => "in error",
        pw::stream::StreamState::Unconnected => "unconnected",
        pw::stream::StreamState::Connecting => "connecting",
        pw::stream::StreamState::Paused => "paused",
        pw::stream::StreamState::Streaming => "streaming",
    }
}

/// Serialize one pod object into the bytes a stream parameter is read from.
///
/// `spa::pod::Object` is the builder form; the PipeWire entry points take the
/// serialized `Pod`, which borrows the bytes it was parsed from.
fn pod_bytes(object: &spa::pod::Object) -> CaptureResult<Vec<u8>> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object.clone()),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|error| {
        CaptureError::platform(anyhow::anyhow!(
            "could not serialize a PipeWire stream parameter: {error}"
        ))
    })
}

/// Build the `SPA_PARAM_EnumFormat` choices offered when connecting.
fn format_objects() -> Vec<spa::pod::Object> {
    OFFERED_FORMATS
        .iter()
        .map(|format| {
            spa::pod::object!(
                spa::utils::SpaTypes::ObjectParamFormat,
                spa::param::ParamType::EnumFormat,
                spa::pod::property!(
                    spa::param::format::FormatProperties::MediaType,
                    Id,
                    spa::param::format::MediaType::Video
                ),
                spa::pod::property!(
                    spa::param::format::FormatProperties::MediaSubtype,
                    Id,
                    spa::param::format::MediaSubtype::Raw
                ),
                spa::pod::property!(
                    spa::param::format::FormatProperties::VideoFormat,
                    Id,
                    *format
                ),
            )
        })
        .collect()
}

/// Ask the compositor for buffers that fit the negotiated frame.
///
/// The size and stride are stated explicitly: a compositor may otherwise hand
/// back buffers sized for a different stride, and the copy below reads whole
/// rows at the stride the buffer reports.
fn buffers_param(info: &VideoInfoRaw) -> spa::pod::Object {
    let bytes_per_pixel = FormatLayout::of(info.format())
        .map(|layout| layout.bytes_per_pixel)
        .unwrap_or(4);
    let stride = info.size().width as i32 * bytes_per_pixel as i32;
    let size = stride * info.size().height as i32;
    spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamBuffers,
        spa::param::ParamType::Buffers,
        spa::pod::property!(SpaKey(spa::sys::SPA_PARAM_BUFFERS_buffers), Int, 8),
        spa::pod::property!(SpaKey(spa::sys::SPA_PARAM_BUFFERS_blocks), Int, 1),
        spa::pod::property!(SpaKey(spa::sys::SPA_PARAM_BUFFERS_size), Int, size),
        spa::pod::property!(SpaKey(spa::sys::SPA_PARAM_BUFFERS_stride), Int, stride),
    )
}

/// The property macros read their key through `as_raw`, which the generated
/// SPA constants do not implement on their own.
struct SpaKey(spa::sys::spa_param_buffers);

impl SpaKey {
    fn as_raw(&self) -> u32 {
        self.0
    }
}

/// The loop thread: connect the remote, create one stream per portal stream and
/// run until [`PortalStreams`] is dropped.
fn run(fd: OwnedFd, shared: Arc<Shared>) {
    pw::init();

    let main_loop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(main_loop) => main_loop,
        Err(error) => {
            shared.report_session_error(format!("could not create a PipeWire main loop: {error}"));
            return;
        }
    };
    let context = match pw::context::ContextRc::new(&main_loop, None) {
        Ok(context) => context,
        Err(error) => {
            shared.report_session_error(format!("could not create a PipeWire context: {error}"));
            return;
        }
    };
    let core = match context.connect_fd_rc(fd, None) {
        Ok(core) => core,
        Err(error) => {
            shared.report_session_error(format!(
                "could not connect to the portal's PipeWire remote: {error}"
            ));
            return;
        }
    };

    let sources = match shared.slots.lock() {
        Ok(slots) => slots
            .iter()
            .map(|slot| slot.source.clone())
            .collect::<Vec<_>>(),
        Err(_) => {
            shared.report_session_error("the capture state was poisoned".to_string());
            return;
        }
    };

    // The listeners own the callbacks, so they have to outlive this function.
    let mut listeners = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        // A stream is addressed by its serial when the portal named one,
        // because node ids are only unique for the lifetime of the graph.
        let (target, flags) = match source.target_object.as_deref() {
            Some(serial) => (None, Some(serial.to_string())),
            None => (Some(source.node_id), None),
        };
        let properties = {
            let mut properties = pw::properties::PropertiesBox::new();
            properties.insert(*pw::keys::MEDIA_TYPE, "Video");
            properties.insert(*pw::keys::MEDIA_CATEGORY, "Capture");
            properties.insert(*pw::keys::MEDIA_ROLE, "Screen");
            if let Some(serial) = flags {
                // `TARGET_OBJECT` only exists behind the crate's version
                // features; the key itself is stable.
                properties.insert("target.object", serial);
            }
            properties
        };

        let stream =
            match pw::stream::StreamRc::new(core.clone(), "snow-shot-screencast", properties) {
                Ok(stream) => stream,
                Err(error) => {
                    shared.report_session_error(format!(
                        "could not create a PipeWire stream: {error}"
                    ));
                    return;
                }
            };

        let state = StreamState {
            index,
            shared: Arc::clone(&shared),
            format: VideoInfoRaw::default(),
            frames: 0,
        };
        let listener = match stream
            .add_local_listener_with_user_data(state)
            .state_changed(|_, state, _, new| {
                portal_debug!("stream {} is {}", state.index, stream_state_name(&new));
                state.set_state(stream_state_name(&new));
                if let pw::stream::StreamState::Error(message) = &new {
                    state.store_error(format!(
                        "the screen cast stream failed: {}",
                        message.as_str()
                    ));
                }
            })
            .param_changed(|stream, state, id, param| {
                let Some(param) = param else {
                    return;
                };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let (media_type, media_subtype) =
                    match spa::param::format_utils::parse_format(param) {
                        Ok(parts) => parts,
                        Err(_) => return,
                    };
                if media_type != spa::param::format::MediaType::Video
                    || media_subtype != spa::param::format::MediaSubtype::Raw
                {
                    return;
                }
                if state.format.parse(param).is_err() {
                    return;
                }
                let info = state.format;
                portal_debug!(
                    "stream {} negotiated {:?} {}x{}",
                    state.index,
                    info.format(),
                    info.size().width,
                    info.size().height
                );
                if FormatLayout::of(state.format.format()).is_none() {
                    state.store_error(format!(
                        "the compositor chose the unsupported video format {:?}",
                        state.format.format()
                    ));
                    return;
                }
                let object = buffers_param(&state.format);
                let bytes = match pod_bytes(&object) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        state.store_error(error.to_string());
                        return;
                    }
                };
                let Some(param) = Pod::from_bytes(&bytes) else {
                    state.store_error("could not read back the PipeWire buffer layout".to_string());
                    return;
                };
                let mut params = [param];
                match stream.update_params(&mut params) {
                    Ok(()) => portal_debug!("stream {} buffers requested", state.index),
                    Err(error) => {
                        portal_debug!("stream {} buffer request failed: {error}", state.index);
                        state.store_error(format!("could not size the PipeWire buffers: {error}"));
                    }
                }
            })
            .process(|stream, state| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let info = state.format;
                let width = info.size().width;
                let height = info.size().height;
                let result = {
                    let datas = buffer.datas_mut();
                    match datas.first_mut() {
                        None => Err(CaptureError::platform(anyhow::anyhow!(
                            "the compositor delivered an empty PipeWire buffer"
                        ))),
                        Some(data) => {
                            let chunk = data.chunk();
                            let offset = chunk.offset() as usize;
                            let stride = chunk.stride().unsigned_abs() as usize;
                            match data.data() {
                                None => Err(CaptureError::platform(anyhow::anyhow!(
                                    "the PipeWire buffer is not mapped"
                                ))),
                                Some(bytes) => copy_rows_to_bgra(
                                    bytes,
                                    offset,
                                    stride,
                                    width,
                                    height,
                                    info.format(),
                                )
                                .and_then(|pixels| Frame::from_bgra8(width, height, pixels)),
                            }
                        }
                    }
                };
                match result {
                    Ok(frame) => {
                        state.frames += 1;
                        if state.frames == 1 || state.frames % 100 == 0 {
                            portal_debug!(
                                "stream {} frame #{} {}x{}",
                                state.index,
                                state.frames,
                                frame.width(),
                                frame.height()
                            );
                        }
                        state.store_frame(frame);
                    }
                    Err(error) => {
                        portal_debug!("stream {} frame failed: {error}", state.index);
                        state.store_error(error.to_string());
                    }
                }
            })
            .register()
        {
            Ok(listener) => listener,
            Err(error) => {
                shared.report_session_error(format!(
                    "could not listen to the PipeWire stream: {error}"
                ));
                return;
            }
        };

        let objects = format_objects();
        let mut serialized: Vec<Vec<u8>> = Vec::with_capacity(objects.len());
        for object in &objects {
            match pod_bytes(object) {
                Ok(bytes) => serialized.push(bytes),
                Err(error) => {
                    shared.report_session_error(error.to_string());
                    return;
                }
            }
        }
        let mut pods: Vec<&Pod> = serialized
            .iter()
            .filter_map(|bytes| Pod::from_bytes(bytes))
            .collect();
        let connect_flags =
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS;
        match stream.connect(Direction::Input, target, connect_flags, &mut pods) {
            Ok(()) => portal_debug!(
                "stream {} connected to node {} as {}",
                index,
                source.node_id,
                stream.node_id()
            ),
            Err(error) => {
                shared.report_session_error(format!(
                    "could not connect the screen cast stream to node {}: {error}",
                    source.node_id
                ));
                return;
            }
        }
        listeners.push((stream, listener));
    }

    // Ask the loop to stop from whichever thread drops the streams: a timer
    // keeps the check cheap and bounded.
    let stop_flag = Arc::clone(&shared);
    let quit_loop = main_loop.clone();
    let timer = main_loop.loop_().add_timer(move |_| {
        if stop_flag.stop.load(Ordering::SeqCst) {
            quit_loop.quit();
        }
    });
    // The timer is only there to notice the stop flag, so a failure to arm it
    // is reported rather than ignored: without it the loop would never end.
    if let Err(error) = timer
        .update_timer(
            Some(Duration::from_millis(50)),
            Some(Duration::from_millis(50)),
        )
        .into_result()
    {
        eprintln!("[snow-portal] could not arm the stop timer: {error}");
    }

    main_loop.run();

    drop(listeners);
    drop(timer);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One BGRx pixel per four bytes, the layout GNOME publishes.
    #[test]
    fn converts_bgrx_rows_to_opaque_bgra() {
        let source = [0x10, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0x7f];
        let pixels =
            copy_rows_to_bgra(&source, 0, 8, 2, 1, VideoFormat::BGRx).expect("the row converts");
        assert_eq!(pixels, vec![0x10, 0x20, 0x30, 0xff, 0x40, 0x50, 0x60, 0xff]);
    }

    #[test]
    fn converts_rgba_rows_to_bgra() {
        let source = [1, 2, 3, 4];
        let pixels =
            copy_rows_to_bgra(&source, 0, 4, 1, 1, VideoFormat::RGBA).expect("the row converts");
        assert_eq!(pixels, vec![3, 2, 1, 4]);
    }

    #[test]
    fn expands_three_byte_rows() {
        let source = [1, 2, 3];
        let pixels =
            copy_rows_to_bgra(&source, 0, 3, 1, 1, VideoFormat::BGR).expect("the row converts");
        assert_eq!(pixels, vec![1, 2, 3, 0xff]);
    }

    /// The compositor may pad rows and offset the first one, so both are
    /// honoured rather than assumed to be tight and zero.
    #[test]
    fn honours_the_chunk_offset_and_stride() {
        let source = [
            0xee, 0xee, // offset
            0x01, 0x02, 0x03, 0x00, 0xaa, 0xaa, // first row and padding
            0x04, 0x05, 0x06, 0x00, 0xaa, 0xaa, // second row and padding
        ];
        let pixels =
            copy_rows_to_bgra(&source, 2, 6, 1, 2, VideoFormat::BGRx).expect("the rows convert");
        assert_eq!(pixels, vec![0x01, 0x02, 0x03, 0xff, 0x04, 0x05, 0x06, 0xff]);
    }

    /// A frame that claims more bytes than the mapped buffer holds is refused
    /// instead of reading past the end of the mapping.
    #[test]
    fn refuses_a_frame_larger_than_the_buffer() {
        let source = [0u8; 8];
        let error = copy_rows_to_bgra(&source, 0, 8, 4, 4, VideoFormat::BGRx)
            .expect_err("an undersized buffer is rejected");
        assert!(
            error.to_string().contains("4x4"),
            "the error names the frame geometry: {error}"
        );
    }

    #[test]
    fn refuses_a_format_it_cannot_convert() {
        let source = [0u8; 16];
        let error = copy_rows_to_bgra(&source, 0, 8, 1, 1, VideoFormat::I420)
            .expect_err("a planar format is rejected");
        assert!(
            error.to_string().contains("unsupported video format"),
            "the error names the format: {error}"
        );
    }

    /// Every offered format has to be convertible, or negotiation would
    /// succeed and the frame would then be dropped.
    #[test]
    fn every_offered_format_is_convertible() {
        for format in OFFERED_FORMATS {
            assert!(
                FormatLayout::of(format).is_some(),
                "{format:?} is offered but cannot be converted"
            );
        }
    }
}
