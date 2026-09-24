//! Screen capture on Wayland through the XDG desktop portal screen cast.
//!
//! A Wayland session cannot be read through X11 (the X root window belongs to
//! XWayland and holds no screen content), and the portal's `Screenshot` method
//! only ever returns one image of the whole desktop: no per-display geometry,
//! no cursor and a permission round trip for every frame. The portal instead
//! offers a screen cast session, which publishes the selected outputs as
//! PipeWire nodes, and that is what this module consumes:
//!
//! 1. register the application id with the host registry, as the portal refuses
//!    an unidentified caller,
//! 2. create a screen cast session, ask for monitor sources and start it (the
//!    compositor shows its own picker the first time),
//! 3. open the session's PipeWire remote and read every published stream, and
//! 4. hand the compositor's geometry for each stream to the capture layer.
//!
//! The session is opened on demand and kept until it has been idle for a while:
//! the compositor displays a screen-sharing indicator for as long as the cast
//! runs, so a screenshot tool must not hold one open forever. The restore token
//! the portal returns is remembered, which is what lets a later session start
//! without showing the picker again.

mod pipewire_stream;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ashpd::AppID;
use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
    Stream as PortalStream,
};
use ashpd::desktop::{PersistMode, Session};
use ashpd::enumflags2::BitFlags;

use crate::backend::{CaptureBackend, CaptureBackendKind, MonitorCapturer};
use crate::capture_session::CaptureTargetInfo;
use crate::error::{CaptureError, CaptureResult};
use crate::frame::{CapturePixelFormat, Frame};
use crate::monitor::MonitorId;
use crate::region::{MonitorGeometry, MonitorLayout};
use crate::window::WindowId;

use self::pipewire_stream::{FIRST_FRAME_TIMEOUT, PortalStreams, StreamSource};

/// The application id the portal stores permissions against. It matches the
/// desktop entry the Linux package installs.
const APP_ID: &str = "com.snowshot.snow_shot";

/// How long a session may stay idle before it is released. The compositor keeps
/// a screen-sharing indicator up while a cast is running, so the session is
/// dropped once a capture has not asked for it for a while; reopening is silent
/// as long as the restore token is still valid.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the portal's restore token is kept between runs.
///
/// It is state rather than user data, so it belongs under `XDG_STATE_HOME`; a
/// test can point `SNOW_CAPTURE_PORTAL_TOKEN_FILE` somewhere disposable.
fn token_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SNOW_CAPTURE_PORTAL_TOKEN_FILE")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("snow-shot").join("portal-screencast-token"))
}

fn read_restore_token() -> Option<String> {
    let path = token_path()?;
    let token = std::fs::read_to_string(&path).ok()?;
    let token = token.trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn store_restore_token(token: &str) {
    let Some(path) = token_path() else {
        return;
    };
    if token.is_empty() {
        return;
    }
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let _ = std::fs::write(path, token);
}

/// One portal screen cast session and the frames it publishes.
struct PortalSession {
    /// Kept alive so the portal keeps the cast running.
    _screencast: Screencast,
    /// Closing the session is what stops the cast, so it lives as long as the
    /// frames are wanted.
    _session: Session<Screencast>,
    sources: Vec<StreamSource>,
    streams: PortalStreams,
    last_use: Mutex<Instant>,
}

impl PortalSession {
    fn open() -> CaptureResult<Self> {
        let restore_token = read_restore_token();
        let (screencast, session, sources, fd, token) =
            futures_lite::future::block_on(open_session(restore_token.as_deref()))?;
        if let Some(token) = token {
            store_restore_token(&token);
        }
        let streams = PortalStreams::start(fd, sources.clone())?;
        Ok(Self {
            _screencast: screencast,
            _session: session,
            sources,
            streams,
            last_use: Mutex::new(Instant::now()),
        })
    }

    fn touch(&self) {
        if let Ok(mut last_use) = self.last_use.lock() {
            *last_use = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_use
            .lock()
            .map(|last_use| last_use.elapsed())
            .unwrap_or_default()
    }

    fn layout(&self) -> MonitorLayout {
        layout_from_sources(&self.sources, None)
    }

    /// The frame of one stream, waiting for it if the cast has not produced one.
    fn capture(&self, index: usize) -> CaptureResult<Frame> {
        self.touch();
        let frames = self.streams.capture(FIRST_FRAME_TIMEOUT)?;
        frames
            .into_iter()
            .nth(index)
            .map(|frame| frame.frame)
            .ok_or_else(|| {
                CaptureError::platform(anyhow::anyhow!(
                    "the screen cast published no stream at index {index}"
                ))
            })
    }
}

/// The portal calls, in the order the screen cast specification requires them.
#[allow(clippy::type_complexity)]
async fn open_session(
    restore_token: Option<&str>,
) -> CaptureResult<(
    Screencast,
    Session<Screencast>,
    Vec<StreamSource>,
    std::os::fd::OwnedFd,
    Option<String>,
)> {
    let app_id = AppID::try_from(APP_ID).map_err(|error| {
        CaptureError::platform(anyhow::anyhow!("invalid portal application id: {error}"))
    })?;
    // Failing to register is not fatal: a portal that does not implement the
    // host registry may still accept the request.
    let _ = ashpd::register_host_app(app_id).await;

    let screencast = Screencast::new().await.map_err(|error| {
        CaptureError::platform(anyhow::anyhow!(
            "no screen cast portal is available: {error}"
        ))
    })?;

    let session = screencast
        .create_session(Default::default())
        .await
        .map_err(|error| {
            CaptureError::platform(anyhow::anyhow!(
                "the portal refused to create a screen cast session: {error}"
            ))
        })?;

    let cursor_mode = match screencast.available_cursor_modes().await {
        Ok(modes) if modes.contains(CursorMode::Embedded) => CursorMode::Embedded,
        _ => CursorMode::Hidden,
    };
    let sources: BitFlags<SourceType> = match screencast.available_source_types().await {
        Ok(types) if types.contains(SourceType::Monitor) => SourceType::Monitor.into(),
        Ok(_) => {
            return Err(CaptureError::platform(anyhow::anyhow!(
                "the screen cast portal does not offer monitor capture"
            )));
        }
        // The property is only available on newer portals; monitor capture is
        // the only source a whole-desktop screenshot can use.
        Err(_) => SourceType::Monitor.into(),
    };

    screencast
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor_mode)
                .set_sources(sources)
                .set_multiple(true)
                .set_restore_token(restore_token)
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await
        .map_err(|error| {
            CaptureError::platform(anyhow::anyhow!(
                "the portal refused the screen cast sources: {error}"
            ))
        })?;

    let response = screencast
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|error| {
            CaptureError::platform(anyhow::anyhow!(
                "the portal refused to start the screen cast: {error}"
            ))
        })?
        .response()
        .map_err(|error| {
            CaptureError::platform(anyhow::anyhow!(
                "the screen cast session did not start: {error}"
            ))
        })?;

    let sources = stream_sources(response.streams());
    if sources.is_empty() {
        return Err(CaptureError::platform(anyhow::anyhow!(
            "the compositor shared no monitor with the screen cast session"
        )));
    }

    let fd = screencast
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|error| {
            CaptureError::platform(anyhow::anyhow!(
                "the portal returned no PipeWire remote: {error}"
            ))
        })?;

    Ok((
        screencast,
        session,
        sources,
        fd,
        response.restore_token().map(str::to_owned),
    ))
}

/// Describe the streams the compositor published.
fn stream_sources(streams: &[PortalStream]) -> Vec<StreamSource> {
    streams
        .iter()
        .map(|stream| StreamSource {
            node_id: stream.pipe_wire_node_id(),
            target_object: None,
            position: stream.position(),
            size: stream.size().and_then(|(width, height)| {
                (width > 0 && height > 0).then_some((width as u32, height as u32))
            }),
        })
        .collect()
}

/// Geometry of the desktop the portal described: the compositor reports each
/// stream's position and size in physical pixels, which is the space the
/// frames arrive in as well.
fn layout_from_sources(sources: &[StreamSource], frame_size: Option<(u32, u32)>) -> MonitorLayout {
    let monitors = sources
        .iter()
        .enumerate()
        .map(|(index, source)| MonitorGeometry {
            monitor: monitor_of(index, source),
            x: source.position.map(|(x, _)| x).unwrap_or(0),
            y: source.position.map(|(_, y)| y).unwrap_or(0),
            width: source
                .size
                .map(|(width, _)| width)
                .or(frame_size.map(|(width, _)| width))
                .unwrap_or(0),
            height: source
                .size
                .map(|(_, height)| height)
                .or(frame_size.map(|(_, height)| height))
                .unwrap_or(0),
        })
        .collect::<Vec<_>>();

    // The portal reports stream positions inside the virtual desktop, so the
    // bounding box of every stream is the extent a caller has to cover.
    let left = monitors.iter().map(|monitor| monitor.x).min().unwrap_or(0);
    let top = monitors.iter().map(|monitor| monitor.y).min().unwrap_or(0);
    let right = monitors
        .iter()
        .map(|monitor| monitor.x + monitor.width as i32)
        .max()
        .unwrap_or(0);
    let bottom = monitors
        .iter()
        .map(|monitor| monitor.y + monitor.height as i32)
        .max()
        .unwrap_or(0);

    MonitorLayout {
        monitors,
        virtual_left: left,
        virtual_top: top,
        virtual_width: (right - left).max(0) as u32,
        virtual_height: (bottom - top).max(0) as u32,
    }
}

/// The capture layer identifies a monitor by a stable id and a name.
///
/// The portal numbers its streams, but those numbers are allocated per session,
/// so the name is built from the geometry the compositor reports instead: a
/// monitor keeps its identity across sessions and only changes when it is moved
/// or resized. A stream without geometry falls back to its node id.
fn monitor_of(index: usize, source: &StreamSource) -> MonitorId {
    let name = match (source.position, source.size) {
        (Some((x, y)), Some((width, height))) => {
            format!("portal:{x},{y}:{width}x{height}")
        }
        (Some((x, y)), None) => format!("portal:{x},{y}"),
        _ => format!("portal:node-{}", source.node_id),
    };
    MonitorId::from_name(source.node_id as isize, name, index == 0)
}

/// The Wayland capture backend: everything comes from one portal screen cast
/// session, opened when the first capture asks for it.
pub(crate) struct PortalBackend {
    session: Arc<Mutex<Option<PortalSession>>>,
}

impl PortalBackend {
    pub(crate) fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Run `use_session`, opening (or reopening) the portal session first.
    fn with_session<T>(
        &self,
        use_session: impl FnOnce(&PortalSession) -> CaptureResult<T>,
    ) -> CaptureResult<T> {
        with_session(&self.session, use_session)
    }
}

impl CaptureBackend for PortalBackend {
    fn enumerate_monitors(&self) -> CaptureResult<Vec<MonitorId>> {
        self.with_session(|session| {
            Ok(session
                .sources
                .iter()
                .enumerate()
                .map(|(index, source)| monitor_of(index, source))
                .collect())
        })
    }

    fn primary_monitor(&self) -> CaptureResult<MonitorId> {
        self.with_session(|session| {
            session
                .sources
                .first()
                .map(|source| monitor_of(0, source))
                .ok_or_else(|| {
                    CaptureError::platform(anyhow::anyhow!(
                        "the screen cast session published no monitor"
                    ))
                })
        })
    }

    fn monitor_layout(&self) -> CaptureResult<MonitorLayout> {
        self.with_session(|session| Ok(session.layout()))
    }

    fn inspect_window(&self, _window: &WindowId) -> CaptureResult<CaptureTargetInfo> {
        Err(CaptureError::platform(anyhow::anyhow!(
            "window inspection is not implemented by the Wayland portal backend"
        )))
    }

    fn create_monitor_capturer(
        &self,
        monitor: &MonitorId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        let index = self.with_session(|session| {
            session
                .sources
                .iter()
                .enumerate()
                .find(|(index, source)| monitor_of(*index, source).key() == monitor.key())
                .map(|(index, _)| index)
                .ok_or_else(|| {
                    CaptureError::InvalidTarget(format!(
                        "{} is not part of the screen cast session",
                        monitor.stable_id()
                    ))
                })
        })?;
        Ok(Box::new(PortalMonitorCapturer {
            session: Arc::clone(&self.session),
            session_index: index,
        }))
    }

    fn create_window_capturer(
        &self,
        _window: &WindowId,
    ) -> CaptureResult<Box<dyn MonitorCapturer>> {
        Err(CaptureError::platform(anyhow::anyhow!(
            "window capture is not implemented by the Wayland portal backend"
        )))
    }
}

/// Captures one monitor of the portal session.
struct PortalMonitorCapturer {
    /// The session the backend opened, shared rather than borrowed so the
    /// capturer keeps it alive on its own.
    session: Arc<Mutex<Option<PortalSession>>>,
    session_index: usize,
}

/// Run `use_session` against the open session, opening (or reopening) it first.
fn with_session<T>(
    session: &Mutex<Option<PortalSession>>,
    use_session: impl FnOnce(&PortalSession) -> CaptureResult<T>,
) -> CaptureResult<T> {
    let mut guard = session.lock().map_err(|_| {
        CaptureError::platform(anyhow::anyhow!("the portal session state was poisoned"))
    })?;
    if let Some(open) = guard.as_ref()
        && open.idle_for() >= IDLE_TIMEOUT
    {
        // Dropping the session closes the cast, which takes the compositor's
        // sharing indicator down with it.
        *guard = None;
    }
    if guard.is_none() {
        *guard = Some(PortalSession::open()?);
    }
    let open = guard.as_ref().expect("the session was just opened");
    use_session(open)
}

impl MonitorCapturer for PortalMonitorCapturer {
    fn backend_kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::Portal
    }

    fn set_output_pixel_format(&mut self, format: CapturePixelFormat) -> CaptureResult<()> {
        match format {
            CapturePixelFormat::Bgra8 => Ok(()),
            CapturePixelFormat::Rgba8 => Err(CaptureError::platform(anyhow::anyhow!(
                "the Wayland portal backend produces BGRA frames only"
            ))),
        }
    }

    fn capture(&mut self, _reuse: Option<Frame>) -> CaptureResult<Frame> {
        with_session(&self.session, |session| session.capture(self.session_index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        node_id: u32,
        position: Option<(i32, i32)>,
        size: Option<(u32, u32)>,
    ) -> StreamSource {
        StreamSource {
            node_id,
            target_object: None,
            position,
            size,
        }
    }

    /// The compositor reports each stream's place on the desktop; the layout
    /// has to carry that through so a region maps back to the right output.
    #[test]
    fn builds_a_layout_from_the_portal_geometry() {
        let sources = vec![
            source(41, Some((0, 0)), Some((1920, 1080))),
            source(42, Some((1920, 0)), Some((1280, 1024))),
        ];
        let layout = layout_from_sources(&sources, None);
        assert_eq!(layout.monitors.len(), 2);
        assert_eq!(
            (layout.monitors[0].x, layout.monitors[0].y),
            (0, 0),
            "the first stream starts at the origin"
        );
        assert_eq!(
            (layout.monitors[1].x, layout.monitors[1].y),
            (1920, 0),
            "the second stream keeps its own position"
        );
        assert_eq!((layout.virtual_width, layout.virtual_height), (3200, 1080));
        assert_eq!((layout.virtual_left, layout.virtual_top), (0, 0));
    }

    /// A stream placed left of the origin still has to produce a positive
    /// extent, and the bounding box has to start where the stream does.
    #[test]
    fn keeps_a_negative_origin_inside_the_bounding_box() {
        let sources = vec![
            source(7, Some((-1280, -120)), Some((1280, 1024))),
            source(8, Some((0, 0)), Some((1920, 1080))),
        ];
        let layout = layout_from_sources(&sources, None);
        assert_eq!((layout.virtual_left, layout.virtual_top), (-1280, -120));
        assert_eq!((layout.virtual_width, layout.virtual_height), (3200, 1200));
    }

    /// The frame size is the fallback when the portal does not describe the
    /// stream, so a layout is never a zero-sized monitor.
    #[test]
    fn falls_back_to_the_frame_size() {
        let sources = vec![source(9, None, None)];
        let layout = layout_from_sources(&sources, Some((800, 600)));
        assert_eq!(
            (layout.monitors[0].width, layout.monitors[0].height),
            (800, 600)
        );
        assert_eq!((layout.virtual_width, layout.virtual_height), (800, 600));
    }

    /// Only the first stream is primary, and the same stream always maps to
    /// the same monitor key so sessions can find it again.
    #[test]
    fn names_monitors_after_their_stream() {
        let first = monitor_of(0, &source(41, Some((0, 0)), Some((1920, 1080))));
        let second = monitor_of(1, &source(42, Some((1920, 0)), Some((1280, 1024))));
        assert!(
            first.is_primary(),
            "the first stream is the primary monitor"
        );
        assert!(!second.is_primary(), "later streams are not primary");
        assert_ne!(first.key(), second.key(), "streams get distinct keys");
        assert_eq!(
            first.key(),
            monitor_of(0, &source(41, Some((0, 0)), Some((1920, 1080)))).key(),
            "the same stream keeps its key across sessions"
        );
        assert!(
            first.name().contains("1920x1080"),
            "the name carries the stream geometry: {}",
            first.name()
        );
    }

    /// The token location has to follow the environment so a test run does not
    /// touch the real one, and so a host that sets XDG_STATE_HOME is honoured.
    #[test]
    fn resolves_the_restore_token_path() {
        // SAFETY: this test owns the process environment it reads; no other
        // test in this crate looks at these variables.
        unsafe {
            std::env::set_var("SNOW_CAPTURE_PORTAL_TOKEN_FILE", "/tmp/snow-token-test");
        }
        assert_eq!(
            token_path(),
            Some(PathBuf::from("/tmp/snow-token-test")),
            "an explicit file wins"
        );

        unsafe {
            std::env::remove_var("SNOW_CAPTURE_PORTAL_TOKEN_FILE");
            std::env::set_var("XDG_STATE_HOME", "/tmp/snow-state-test");
        }
        assert_eq!(
            token_path(),
            Some(PathBuf::from(
                "/tmp/snow-state-test/snow-shot/portal-screencast-token"
            )),
            "the token lives under XDG_STATE_HOME"
        );
        unsafe {
            std::env::remove_var("XDG_STATE_HOME");
        }
    }
}
