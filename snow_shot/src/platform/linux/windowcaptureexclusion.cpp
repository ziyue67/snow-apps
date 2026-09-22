#include "snow_shot/platform/windowcaptureexclusion.h"

#include <QGuiApplication>
#include <QString>
#include <QThread>
#include <QWidget>

namespace snow_shot::platform {
namespace {

// X11 exposes no per-window capture-exclusion protocol, and Wayland only offers
// one to privileged portal clients. Snow Shot never depends on exclusion for
// correctness: the capture surfaces hide themselves before a frame is taken, so
// reporting the request as unsupported keeps the Windows and macOS contract
// without failing the capture path.
bool supportsCaptureExclusion() {
    return false;
}

// Only the X11 platform plugins publish a compositor window identifier that
// clients may read back. Wayland surfaces have no such client-visible handle.
bool isX11Session() {
    const QString platform = QGuiApplication::platformName();
    return platform == QStringLiteral("xcb") || platform == QStringLiteral("xlib");
}

} // namespace

bool setWindowExcludedFromCapture(QWidget* window, bool excluded) {
    if (window == nullptr || QThread::currentThread() != window->thread()) {
        return false;
    }
    if (!excluded) {
        // Nothing is ever excluded on this platform, so clearing always succeeds
        // and the caller does not have to track platform-specific state.
        return true;
    }
    return supportsCaptureExclusion();
}

std::optional<std::uint32_t> captureWindowId(QWidget* window) {
    if (window == nullptr || QThread::currentThread() != window->thread()) {
        return std::nullopt;
    }
    if (!isX11Session()) {
        return std::nullopt;
    }
    QWidget* topLevel = window->window();
    if (topLevel == nullptr) {
        return std::nullopt;
    }
    const WId handle = topLevel->winId();
    if (handle == 0) {
        return std::nullopt;
    }
    return static_cast<std::uint32_t>(handle);
}

} // namespace snow_shot::platform
