# Linux Compatibility Audit

This audit targets the Ubuntu/Tauri branch used by the Debian package in this fork.
The upstream project's documented development platform is Windows 11, so Linux
compatibility is maintained here as a downstream port.

## Status

| Area | Linux status | Notes |
| --- | --- | --- |
| Application startup and settings UI | Verified on earlier package | Tauri/WebKit UI starts on Ubuntu 26.04. |
| Tray menu | Verified on earlier package | Uses libayatana-appindicator. |
| Screenshot selection UI | Verified on Ubuntu 26.04 | GNOME Wayland screenshot capture showed real screen content instead of a blank image. |
| OCR | Packaged, runtime verification pending | ONNX Runtime 1.20.0 is bundled in the deb and linked at install time. |
| Primary-selection text capture | Implemented, runtime verification pending | Uses `wl-paste` or `xclip`, then falls back to the Rust helper. |
| Translation and AI chat UI | Verified on Ubuntu 26.04 | `你好` was translated to `Hello` through the Linux UI and the Youdao fallback provider. |
| Global hotkeys | Partial | Works on X11; GNOME Wayland may reject global shortcut registration. |
| Focused-window capture | Partial | Uses X11 active-window lookup and xcap window capture; native Wayland windows may fall back to monitor capture. |
| Always-on-top | Partial | Uses `wmctrl` on X11; native Wayland support depends on the compositor. |
| Free window drag | Implemented and build-verified | Uses the Tauri window drag API on Linux. |
| Desktop notification | Implemented and build-verified | Uses the freedesktop notification service through `notify-rust`. |
| Scroll screenshot | Improved | Uses the screenshot backend plus AT-SPI/window-level element discovery. |
| Video recording | Partial | Uses system FFmpeg and `x11grab`/`pulse`; native Wayland screen capture is not complete. |
| Multi-monitor | Partial | Monitor enumeration exists, but element-level selection is window-level only. |
| Element-level UI automation | Implemented and build-verified | Uses Linux AT-SPI to collect accessible element rectangles, with window-level fallback. |

## Runtime dependencies

The Debian package declares the following Linux integration packages:

- `xdotool`: active-window lookup
- `wmctrl`: always-on-top window state
- `wl-clipboard` or `xclip`: primary-selection text capture
- `ffmpeg`: video recording
- `libxdo3`, `libgbm1`, and PipeWire libraries

## Verification

The GitHub Actions workflow builds the deb, verifies the bundled ONNX Runtime,
checks the generated `postinst`/`prerm` scripts, and runs `ldd` against the
packaged binary before publishing the release.

Manual verification performed on Ubuntu 26.04:

- Installed the generated deb and confirmed `ldd /usr/bin/app` has no missing libraries.
- Triggered screenshot capture from the tray menu; the selection window displayed the real desktop content.
- Opened the Translation page and confirmed the source/target/service controls render correctly.
- Translated `你好` to `Hello` through the UI using the fallback Youdao provider.
