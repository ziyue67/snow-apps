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
    libgl1-mesa-dev libvulkan-dev \
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
    libopencv-dev libonnxruntime-dev libminizip-ng-dev \
    libpng-dev libjpeg-turbo8-dev libturbojpeg0-dev libwebp-dev libheif-dev \
    libjxl-dev libgif-dev libopenexr-dev \
    libxcb-cursor0 libxcb-icccm4 libxcb-image0 libxcb-keysyms1 libxcb-randr0 \
    libxcb-render-util0 libxcb-shape0 libxcb-sync1 libxcb-xfixes0 \
    libxcb-xinerama0 libxcb-xkb1 libxkbcommon-x11-0 libfontconfig1 libfreetype6 \
    libglx-mesa0 libopengl0
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
    --archives qtbase qtsvg qttools qttranslations icu qtwayland
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

## Known limitations

The Linux port currently covers the build system, the platform shims and the
packaging path. The following behaviour is not implemented yet and is tracked as
follow-up work:

- **Screen capture.** `snow-capture` has no Linux backend yet; the desktop
  capture entry points report the platform as unsupported. `src/platform/mod.rs`
  currently answers every non-Windows, non-macOS request with an
  `UnsupportedBackend`. A Linux backend has to provide the `CaptureBackend`
  surface (`enumerate_monitors`, `primary_monitor`, `monitor_layout`,
  `inspect_window`, `create_monitor_capturer`, `create_window_capturer`), a
  `MonitorCapturer` implementation, and `monitor_layout_from_monitors`. Only
  `MonitorCapturer::capture` has to be written: `set_cancellation`,
  `set_cursor_visible`, `set_screen_color_transform`, `backend_kind` and
  `prewarm_environment` all have default implementations, and `prewarm_environment`
  must not leave a capture session open when it returns. On X11
  that means monitor enumeration through RandR, capture through `XGetImage` or
  shared-memory `XShm`, and conversion of the returned image into the crate's
  frame format; window capture additionally needs `XComposite`. Wayland needs a
  portal/PipeWire path instead. Until then the offline preview and editing paths
  still work, but nothing can acquire the screen. Capability reporting already
  degrades correctly: `CaptureCapabilities::current()` takes the
  `cfg!(windows)` branch, so Linux advertises an empty `backends` and
  `cpu_formats` list and capture attempts return an error instead of promising
  support, which leaves the remaining work as the backend itself rather than the
  reporting around it.
- **Tray icon warning.** Starting the packaged build prints
  `QObject::connect: No such signal QPlatformNativeInterface::systemTrayWindowChanged(QScreen*)`.
  The string lives only in Qt's own `libQt6Widgets.so.6` (three occurrences) and
  in neither the application binary nor `libQt6Gui`/`libqxcb`, so Qt's
  `QSystemTrayIcon` is connecting to a platform-native signal the XCB plugin no
  longer provides. It is a Qt-internal mismatch rather than an application
  defect, and the connect failure is harmless; confirming tray behaviour needs a
  real desktop session with a status-notifier host.
- **Global shortcuts, global mouse, XDG auto-start, physical cursor and
  focused-fullscreen detection.** These platform services still resolve to the
  unsupported stubs, so the corresponding features stay off.
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
