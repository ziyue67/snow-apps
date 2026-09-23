# Building Snow Shot on Linux

Snow Shot targets Linux with the Qt 6 Widgets stack. The build resolves its
native dependencies from the distribution package manager instead of vcpkg, and
the Debian package is produced from the same CMake install rules the other
platforms use.

## Prerequisites

Ubuntu 26.04 (or a distribution with comparable package versions) needs the
following packages. The Qt modules, FFmpeg, OpenCV and ONNX Runtime all come
from the distribution.

```bash
sudo apt-get install -y --no-install-recommends \
    build-essential ninja-build cmake pkg-config clang-format clang libclang-dev \
    file libssl-dev \
    libgl1-mesa-dev libvulkan-dev \
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
    libopencv-dev libonnxruntime-dev libminizip-ng-dev \
    libpng-dev libjpeg-turbo8-dev libturbojpeg0-dev libwebp-dev libheif-dev \
    libjxl-dev libgif-dev libopenexr-dev \
    libx11-dev libxi-dev libdbus-1-dev libxkbcommon-dev \
    libxcb-cursor0 libxcb-icccm4 libxcb-image0 libxcb-keysyms1 libxcb-randr0 \
    libxcb-render-util0 libxcb-shape0 libxcb-sync1 libxcb-xfixes0 \
    libxcb-xinerama0 libxcb-xkb1 libxkbcommon-x11-0 libfontconfig1 libfreetype6 \
    libglx-mesa0 libopengl0 \
    libwayland-client0 libwayland-cursor0 libwayland-server0
```

The Rust toolchain is pinned by `rust-toolchain.toml` (1.97.1). Install it with
`rustup` and make sure `~/.cargo/bin` is on `PATH`.

### Qt 6.11.1

The build requires Qt **6.11.1 exactly**, which is newer than most distribution
packages. The repository keeps an aqt-installed kit under `.tools/Qt`:

```bash
python3 -m venv .tools/aqt-venv
.tools/aqt-venv/bin/pip install --upgrade pip aqtinstall
.tools/aqt-venv/bin/aqt install-qt linux desktop 6.11.1 linux_gcc_64 \
    -O .tools/Qt -b https://download.qt.io \
    --archives qtbase qtsvg qttools qttranslations icu qtwayland qtdeclarative
```

Set `Qt6_DIR` to a different kit if you prefer one, for example a distribution
build of a matching version:

```bash
export Qt6_DIR=/path/to/Qt/6.11.1/gcc_64/lib/cmake/Qt6
```

### libjpeg-turbo 3.x

The JPEG codec uses the libjpeg-turbo 3.x handle API (`tj3Init`, `tj3Decompress8`
and the `TJPARAM_*` accessors). Distributions that still ship 2.x need a 3.x
build; the configure step fails with an explicit message when only 2.x is
available. Build one into `.tools/libjpeg-turbo`:

```bash
curl -L -o libjpeg-turbo-3.1.2.tar.gz \
    https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/3.1.2/libjpeg-turbo-3.1.2.tar.gz
tar xzf libjpeg-turbo-3.1.2.tar.gz
cmake -S libjpeg-turbo-3.1.2 -B libjpeg-turbo-3.1.2/build -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DWITH_TURBOJPEG=ON \
    -DCMAKE_INSTALL_PREFIX="$PWD/.tools/libjpeg-turbo"
cmake --build libjpeg-turbo-3.1.2/build -j"$(nproc)"
cmake --install libjpeg-turbo-3.1.2/build
```

The `linux-base` preset already puts `.tools/libjpeg-turbo` on
`CMAKE_PREFIX_PATH`, so nothing else is required.

### Offline or restricted networks

The WeChat QR models are downloaded at configure time from a pinned revision.
When `raw.githubusercontent.com` is unreachable, point the download at a mirror;
the SHA-512 of every file is still verified.

```bash
cmake --preset snow-shot-linux-x64-release \
    -D SNOW_SHOT_QR_MODEL_BASE_URL=https://<mirror>/<pinned-revision-path>
```

## Presets

| Preset | Purpose |
| --- | --- |
| `snow-shot-linux-x64-debug` | Debug build with the test suite enabled. |
| `snow-shot-linux-x64-release` | Release build that configures the Debian package. |
| `snow-shot-linux-x64-fast` | Release build with LTO disabled, for fast iteration. |

```bash
cmake --preset snow-shot-linux-x64-release
cmake --build --preset build-snow-shot-linux-x64-release --target snow_shot --parallel
```

### clang-format is required

Configure regenerates the Ant Design icon packs and refuses to continue when
`clang-format` is missing, because the generated sources must match the
repository formatting.

## Packaging

`scripts/package-snow-shot-linux.sh` configures the release preset, builds the
application, stages `cmake --install` under a scratch root with `DESTDIR`, and
runs CPack with the `DEB` generator declared by the
`package-snow-shot-linux-x64-release` preset.

```bash
scripts/package-snow-shot-linux.sh
```

The staged prefix is `/usr`, so the packaged paths are exactly the paths the
application expects at runtime. Shared-library dependencies are computed by
`dpkg-shlibdeps` rather than hard-coded, which keeps the dependency list correct
across the Debian and Ubuntu package name differences.

```bash
dpkg-deb --info build/snow-shot-linux-x64-release/*.deb
dpkg-deb --contents build/snow-shot-linux-x64-release/*.deb
sudo apt-get install -y ./build/snow-shot-linux-x64-release/*.deb
ldd /usr/bin/snow_shot
```

## Verification

`scripts/verify-snow-shot-linux.sh` runs the whole chain and fails on the first
problem. It configures and builds through the packaging entry point, inspects
the archive, checks that the executable, the codec backend, the bundled Qt
runtime and the platform plugins are all present, confirms every dependency
resolves with the build host Qt removed from the picture, and finally starts
the packaged application on the real X11 path under a virtual display. Run it
after changing the build system, the packaging rules or the startup path:

```bash
scripts/verify-snow-shot-linux.sh
# verify: ok - release preset configured, built and packaged
# verify: ok - archive metadata parses (snow-shot_1.0.9-beta_amd64.deb, … bytes)
# verify: ok - layout carries the executable, codec backend, … Qt libraries and … platform plugins
# verify: ok - the packaged dependencies resolve without the build host Qt
# verify: ok - the packaged application starts on the xcb platform under a virtual X display
# verify: note - screen capture is reported unsupported on Linux; see docs-linux-build.md
# verify: all checks passed
```

It uses the `xcb` plugin rather than `offscreen`, because offscreen hides
platform and plugin failures; install `xvfb` for that step. Without it the run
falls back to offscreen and says so instead of reporting the display path as
proven. Individual checks:

```bash
# Nothing in the package is missing a shared library, including the plugins.
ldd /usr/bin/snow_shot | grep -c 'not found'
ldd /usr/lib/snow-shot/plugins/platforms/libqxcb.so | grep -c 'not found'

# The application starts on the real display path.
xvfb-run -a /usr/bin/snow_shot

# The generated dependencies resolve. Use --reinstall, otherwise apt short
# circuits as soon as the package is already installed and never solves them.
sudo apt-get install --dry-run --reinstall --no-install-recommends ./build/snow-shot-linux-x64-release/*.deb

# The desktop entry is valid and its icon and command resolve.
desktop-file-validate /usr/share/applications/com.snowshot.snow_shot.desktop
```

## Linux capture backend

`src/platform/linux.rs` implements screen capture over X11, and
`src/platform/mod.rs` selects it under `cfg(target_os = "linux")`. The design
points, all verified against the workspace:

- Xlib is bound directly rather than through a crate. `libX11.so` is present
  with its development symlink, so a `#[link(name = "X11")] unsafe extern "C"`
  block covers the handful of calls needed: no new dependency, no registry
  fetch.
- `Frame::from_bgra8(width, height, data)` matches what `XGetImage` returns for
  a 32-bit TrueColor `ZPixmap` (each pixel is `0x00RRGGBB`, which reads back as
  BGRA on a little-endian host), so the captured buffer needs no swizzle. Rows
  are copied with the `bytes_per_line` stride before `XDestroyImage` releases
  the server buffer.
- `XInitThreads` runs once through a `Once`, because capture happens on worker
  threads and Xlib is not thread-safe before that call.

`CaptureBackendKind` gained an `X11` variant, and the C ABI carries it as
`SNOW_CAPTURE_BACKEND_X11 = 5` in `snow_capture.h` alongside the matching
`parse_capture_backend`/`capture_backend_value` arms.

Linking needs care on Linux: a static archive only records that it needs Xlib,
so the final link has to name it. It is attached to the
`snow_shot_rust_ffi_bundle` interface target rather than to `snow_shot`
directly, because linking happens left to right and `-lX11` placed before the
archive resolves nothing.

`tests/linux_capture.rs` covers enumeration, layout and a real captured frame;
the tests skip when `DISPLAY` is unset and assert exact geometry under
`xvfb-run -a -s "-screen 0 320x240x24"`. `scripts/verify-snow-shot-linux.sh`
runs them, so a capture regression fails verification.

Still open: per-output enumeration through RandR (the X screen is currently
reported as one monitor spanning the whole screen), window capture through
XComposite, and a Wayland portal/PipeWire path.

## Linux global shortcuts

`src/platform/linux/globalshortcutbackend.cpp` implements the backend over X11
and `createPlatformGlobalShortcutBackend()` selects it under `Q_OS_LINUX`.
Before that, hotkeys fell through to `UnsupportedGlobalShortcutBackend` on every
platform that is not Windows or macOS and were registered against a no-op, so
they failed silently.

The backend satisfies the four methods of `GlobalShortcutBackend`:

```cpp
using ActivationHandler = std::function<void(int)>;
virtual void setActivationHandler(ActivationHandler handler) = 0;
[[nodiscard]] virtual GlobalShortcutValidationResult
validateShortcut(const snow_shot::shortcuts::ShortcutBinding& binding) const = 0;
[[nodiscard]] virtual GlobalShortcutBackendResult
registerShortcut(int registrationId, const snow_shot::shortcuts::ShortcutBinding& binding) = 0;
virtual void unregisterShortcut(int registrationId) = 0;
```

The data it works with:

- `ShortcutBinding { QString portableText; QMap<ShortcutPlatform, quint32> physicalKeys; }`.
  `ShortcutPlatform` currently only has `MacOS`, so Linux should parse
  `portableText` with `QKeySequence::fromString` rather than expect a
  pre-computed native key.
- `GlobalShortcutBackendResult { bool registered; GlobalShortcutFailureReason failureReason;
  qint64 nativeErrorCode; }` and
  `GlobalShortcutValidationResult { QString shortcut; bool supported;
  GlobalShortcutFailureReason failureReason; ShortcutBinding binding; }`.

Suggested approach, mirroring how the capture backend avoids depending on Qt's
private X connection: open a dedicated display on a worker thread, translate the
key sequence to a keysym and keycode (`XKeysymToKeycode`), map Qt's modifiers to
`ControlMask`, `ShiftMask`, `Mod1Mask` (Alt) and `Mod4Mask` (Meta), and grab on
the root window with `XGrabKey`. Grab the NumLock and CapsLock permutations as
well, or the shortcut stops firing while either lock is active. Then loop on
`XNextEvent`, match `KeyPress` by keycode and state, and hand the registration id
back to the caller: the handler must reach the main thread (a `QObject` context
with a queued invocation) because the manager updates UI in response.

Wayland has no equivalent of `XGrabKey`: a grab lands on the XWayland root, so
registration reports success while the shortcut only fires when an X11 client
holds focus. `src/platform/linux/waylandglobalshortcuts.cpp` goes through the
desktop portal instead — `CreateSession`, then `BindShortcuts`, then the
session's `Activated` signal, marshalled back to the main thread because the
manager updates the UI in response — and `createPlatformGlobalShortcutBackend()`
selects it whenever `WAYLAND_DISPLAY` is set. The compositor may refuse the
request, and the portal has no unbind, so unregistering only drops the local
mapping.

Two details are load-bearing. The `a(sa{sv})` argument needs a declared type
registered through `qDBusRegisterMetaType`; streaming the array by hand left
libdbus expecting `au` and aborting mid-message. And `Q_DECLARE_METATYPE` has to
precede the first `qMetaTypeId<>` use, or the release preset rejects the late
specialisation — the debug preset happened to instantiate in the other order and
built anyway, which is why that one is worth knowing.

Verifying it needs a portal, so `tests/mock_portal.py` serves `GlobalShortcuts`
alongside `Screenshot` on a private bus, and an opt-in check in
`tests/global_shortcut_backend_tests.cpp` binds a shortcut and asserts the
activation arrives with its registration id:

```bash
cmake --build --preset build-snow-shot-linux-x64-debug \
    --target snow-shot-global-shortcut-backend-tests
dbus-run-session -- bash -c \
    "python3 snow_shot/tests/mock_portal.py & sleep 2; \
     SNOW_SHOT_TEST_PORTAL=1 \
     ./build/snow-shot-linux-x64-debug/snow_shot/test-bin/snow-shot-global-shortcut-backend-tests"
```

Without the mock the same binary still checks that an X11 session accepts an
ordinary shortcut and that the X11 backend refuses one on a Wayland session;
neither check needs a display.

## Wayland capture

A Wayland session cannot be read through X11: the X root window belongs to
XWayland and holds no screen content, so `XGetImage` returns blank frames. The
capture worker therefore asks
`org.freedesktop.portal.Screenshot` when `WAYLAND_DISPLAY` is set
(`src/platform/linux/portalscreenshot.cpp`), waits for the request object's
`Response` signal, and turns the returned image into the captured display. The
portal result arrives asynchronously — the method only returns a request handle —
so the signal is subscribed to before waiting, and when the portal refuses or the
user dismisses the prompt its own reason is reported.

Verifying it needs a portal, which a container does not have, so the D-Bus
exchange is covered separately from the session itself:

```bash
cmake --preset snow-shot-linux-x64-debug
cmake --build --preset build-snow-shot-linux-x64-debug \
    --target snow-shot-portal-screenshot-tests
dbus-run-session -- bash -c \
    "python3 snow_shot/tests/mock_portal.py & sleep 2; \
     SNOW_SHOT_TEST_PORTAL=1 \
     ./build/snow-shot-linux-x64-debug/snow_shot/test-bin/snow-shot-portal-screenshot-tests"
```

`tests/mock_portal.py` serves the same interface on a private bus and answers
with a generated PNG, so the round-trip is reproducible without a desktop
session. Two traps it documents: the `Response` signal has to be declared on a
class instantiated at the request path, and the well-known name has to stay
referenced or it is released and the call reaches the real portal instead.

Without the mock the same binary still checks the two things that hold anywhere:
that `portalScreenshotRequired()` follows the session type, and that a failed
helper always explains itself. That check is gated on the session bus being
reachable rather than poisoning `DBUS_SESSION_BUS_ADDRESS`, because
`QDBusConnection` caches its session connection and a forced failure would leave
that broken connection cached for the rest of the process.

Not covered: taking a screenshot in a real Wayland session. That still needs a
desktop to confirm.

## Known limitations

The Linux port currently covers the build system, the platform shims and the
packaging path. The following behaviour is not implemented yet and is tracked as
follow-up work:

- **Screen capture scope.** Whole-screen capture works; per-monitor enumeration
  and window capture do not, so `inspect_window` and `create_window_capturer`
  report an unsupported-platform error. Multi-head setups are seen as one
  monitor covering the X screen, and the portal reports the whole desktop as a
  single display. A Wayland session goes through `org.freedesktop.portal.Screenshot`
  (see above) rather than X11.
  `CaptureCapabilities::current()` reports the X11 backend with BGRA CPU frames
  on Linux, and advertises neither native frames nor window enumeration, so the
  application sees exactly what the backend provides. It still advertises
  nothing on a Wayland session, because the capabilities probe has not been
  taught about the portal path yet, and the direct-capture entry point
  (`captureDirectTarget`) has no portal fallback either: the portal returns the
  whole desktop, which cannot satisfy a focused-window request and only
  approximates a single monitor on a mixed-DPI multi-head setup.
- **Tray icon warning.** Starting the packaged build prints
  `QObject::connect: No such signal QPlatformNativeInterface::systemTrayWindowChanged(QScreen*)`.
  The string lives only in Qt's own `libQt6Widgets.so.6` (three occurrences) and
  in neither the application binary nor `libQt6Gui`/`libqxcb`, so Qt's
  `QSystemTrayIcon` is connecting to a platform-native signal the XCB plugin no
  longer provides. It is a Qt-internal mismatch rather than an application
  defect, and the connect failure is harmless; confirming tray behaviour needs a
  real desktop session with a status-notifier host.
- **No platform service is left as a stub.** All seven have Linux bodies:
  shortcuts are grabbed through X11 (`XGrabKey`, with the Wayland guard above) or
  brokered through the portal, the cursor is read and warped through X11, the
  global mouse is watched with XInput2 raw events on the root window, auto-start
  writes an XDG desktop entry, and fullscreen detection reads EWMH state.

  Two of those are verified only against a stand-in. The portal paths ran against
  the mock portal, and the pointer backend runs offscreen, where there is no X11
  display — so it covers the failure path and the shared gesture contract rather
  than live pointer input. A real session is what confirms either.
- **The bundled Qt runtime ships without its own licence text.** The package
  carries the application's `COPYRIGHT` and GPL-3.0 `LICENSE`, both third-party
  notice documents, and a `COPYRIGHT`/`LICENSE` pair per repository component
  under `/usr/share/snow-shot/licenses/`, but nothing covers the Qt and ICU
  libraries installed beside it. The Qt kit provides no licence files to
  install, so closing this means sourcing Qt's LGPL-3.0/GPL-3.0 text deliberately
  rather than copying it, which is why it is recorded here instead of guessed at.
- **Auto-update.** The release contract is Windows-only; the updater is not
  built for Linux.
- **Asset location.** The install rules put `assets/` and `audios/` below
  `/usr/bin`, because the application resolves them relative to
  `applicationDirPath()`. This works but is not FHS-clean; moving the executable
  to `/usr/lib/snow-shot` with a launcher symlink would require the codec
  backend lookup to move with it.
- **Qt kit completeness.** An aqt kit that omits `qtdeclarative` ships an
  `lrelease` that cannot resolve `libQt6Qml`. Install the `qtdeclarative`
  archive, or make `lrelease` resolve against a matching Qt build.
- **Generated dependencies still name the distribution Qt.** The package bundles
  the Qt 6.11.1 runtime, ICU and the platform plugins under `lib/snow-shot`, so
  it loads and starts on a host that has no Qt 6.11.1 of its own. Those libraries
  are private to the package, so `shlibdeps` cannot infer them and falls back to
  the distribution Qt packages (6.10.2 on the reference system); the result
  over-declares what the bundled build needs, and the dependency list should be
  tightened once the supported distribution matrix is settled.

## Build configuration notes

Two Linux-specific build settings are deliberate rather than incidental:
- **Interprocedural optimization is off.** With `-flto` the linker emits copy
  relocations for Qt and libstdc++ data symbols (`QGuiApplication`'s
  `staticMetaObject`, typeinfo, vtables). The copied symbols make Qt bind its
  own internal references to a stale snapshot, which crashes `QApplication`
  construction before `main` runs. `SNOW_APPS_ENABLE_RELEASE_OPTIMIZATION` is
  therefore applied only outside Linux.
- **GCC-only diagnostics are relaxed.** GCC reports several classes of warnings
  that neither Clang nor MSVC emits, so the sources were never written to
  satisfy them: `-Wmissing-declarations` for C++, and `-Wshadow`,
  `-Wuseless-cast`, `-Wduplicated-branches`, `-Wduplicated-cond`, `-Wlogical-op`,
  `-Wchanges-meaning`, `-Wsubobject-linkage`, `-Wclobbered` and
  `-Wnull-dereference` (the last two surface only once LTO is off, and the
  `-Wnull-dereference` hits are inside Qt and libstdc++ headers).
- **The bundled Qt plugin directory is registered at startup.** The package
  installs the Qt runtime under `lib/snow-shot/lib` and the platform plugins
  under `lib/snow-shot/plugins/platforms`. The `lib` level matters: the plugins
  carry a `$ORIGIN/../../lib` runpath, which only resolves to the bundled
  runtime from that depth — one level higher and the plugins pick up the system
  Qt instead. Qt itself searches only `<applicationDirPath>/platforms`, so
  `main` calls `QCoreApplication::addLibraryPath` with the plugin directory
  before constructing `QApplication`; the executable location comes from
  `/proc/self/exe` because `applicationDirPath()` needs a live instance.
  Installing a `qt.conf` or a `platforms/` directory into `/usr/bin` was
  rejected: every Qt application in that directory would read it.

## Wayland overlay geometry and HiDPI scaling

The capture overlay maps the portal's screenshot into the window with
`canvasToLogicalScale = logicalRect.width / canvasRect.width`
(`ScreenshotGeometryMapper::displayViewportGeometry`). The portal returns the
screen in **physical** pixels, while the overlay window lives in **logical**
coordinates, so the two only agree when the screen scale is taken into account.

Measured on a 1920x1080 panel driven at 200% (GNOME on Wayland, Qt 6.11.1,
`screen=eDP-1`), instrumenting the mapper gave:

```
OVERLAY-GEOM logical=960x540  canvas=1920x1080 scale=0.5000 screen=eDP-1 dpr=2.000
```

So at 200% the logical desktop is 960x540 and a scale of 0.5 is correct: the
1920x1080 capture is drawn at half size to fill the 960x540 logical window. A
"low resolution overlay" here is the logical size, not a rendering fault.

Two consequences worth remembering:

* `org.gnome.desktop.interface scaling-factor` reports the **X11** setting and
  returned `1` on this machine even though the session ran at 200%. Read
  `QScreen::devicePixelRatio()` instead when reasoning about scale.
* Anything that mixes the two spaces is off by the scale factor. The colour
  picker was observed reporting `X: 1909` while logical coordinates only span
  `0..959`, so cursor and selection positions need to be resolved in one space
  consistently while capture stays in physical pixels.

Frameless `Qt::Tool` overlays cannot be positioned with `setGeometry()` on
Wayland — the compositor owns toplevel geometry, so the placement has to be
delegated to it (see `mark-shot`'s `showFullScreenOnScreen`, which on Linux does
only `setScreen` + `setGeometry(screen->geometry())` + `showFullScreen()`).
