#include "snow_shot/platform/focusedfullscreenwindow.h"

#include <algorithm>

#if defined(Q_OS_LINUX)
#include <X11/Xatom.h>
#include <X11/Xlib.h>

#include <QByteArray>

// Xlib's None macro is what the property calls below compare against, so it
// stays defined here.
#endif
#include <cmath>

namespace snow_shot::platform {
namespace {
bool coordinateMatches(qreal first, qreal second) {
    return std::abs(first - second) <= 1.0;
}

bool coversDisplay(const QRectF& window, const QRectF& display) {
    return coordinateMatches(window.left(), display.left()) &&
           coordinateMatches(window.top(), display.top()) &&
           coordinateMatches(window.right(), display.right()) &&
           coordinateMatches(window.bottom(), display.bottom());
}
} // namespace

bool focusedWindowCoversDisplay(qint64 frontmostProcessId,
                                const QVector<FocusedWindowSnapshot>& orderedWindows,
                                const QVector<QRectF>& displayBounds) {
    if (frontmostProcessId <= 0 || displayBounds.isEmpty()) {
        return false;
    }
    const auto focused = std::find_if(
        orderedWindows.cbegin(), orderedWindows.cend(), [frontmostProcessId](const auto& window) {
            return window.ownerProcessId == frontmostProcessId && window.layer == 0 &&
                   window.alpha > 0.0 && window.bounds.isValid() && !window.bounds.isEmpty();
        });
    return focused != orderedWindows.cend() &&
           std::any_of(displayBounds.cbegin(), displayBounds.cend(),
                       [focused](const QRectF& display) {
                           return display.isValid() && !display.isEmpty() &&
                                  coversDisplay(focused->bounds, display);
                       });
}

#if defined(Q_OS_LINUX)
// EWMH only: ask the window manager which window has focus, then whether that
// window advertises the fullscreen state. Everything here is best effort, so a
// missing property or an absent display simply reports "no".
namespace {
// Format 32 properties come back as an array of long, not of int.
long readLongProperty(Display* display, Window window, Atom property, Atom expectedType,
                      int maximumItems) {
    Atom actualType = None;
    int actualFormat = 0;
    unsigned long itemCount = 0;
    unsigned long bytesAfter = 0;
    unsigned char* data = nullptr;
    const int status = XGetWindowProperty(display, window, property, 0, maximumItems, False,
                                          expectedType, &actualType, &actualFormat, &itemCount,
                                          &bytesAfter, &data);
    if (status != Success || actualType != expectedType || actualFormat != 32 || data == nullptr ||
        itemCount == 0) {
        if (data != nullptr) {
            XFree(data);
        }
        return 0;
    }
    const long value = reinterpret_cast<const long*>(data)[0];
    XFree(data);
    return value;
}

bool windowIsFullscreen(Display* display, Window window) {
    if (window == 0) {
        return false;
    }
    const Atom stateProperty = XInternAtom(display, "_NET_WM_STATE", True);
    const Atom fullscreenAtom = XInternAtom(display, "_NET_WM_STATE_FULLSCREEN", True);
    if (stateProperty == None || fullscreenAtom == None) {
        return false;
    }

    Atom actualType = None;
    int actualFormat = 0;
    unsigned long itemCount = 0;
    unsigned long bytesAfter = 0;
    unsigned char* data = nullptr;
    const int status =
        XGetWindowProperty(display, window, stateProperty, 0, 32, False, XA_ATOM, &actualType,
                           &actualFormat, &itemCount, &bytesAfter, &data);
    if (status != Success || actualType != XA_ATOM || actualFormat != 32 || data == nullptr) {
        if (data != nullptr) {
            XFree(data);
        }
        return false;
    }

    const auto* atoms = reinterpret_cast<const long*>(data);
    bool found = false;
    for (unsigned long index = 0; index < itemCount && !found; ++index) {
        found = atoms[index] == static_cast<long>(fullscreenAtom);
    }
    XFree(data);
    return found;
}
} // namespace

bool focusedFullscreenWindowExists() {
    // XWayland only sees X11 clients, so it cannot say whether a Wayland window
    // is fullscreen; report nothing rather than guessing.
    if (!qgetenv("WAYLAND_DISPLAY").isEmpty()) {
        return false;
    }
    Display* display = XOpenDisplay(nullptr);
    if (display == nullptr) {
        return false;
    }
    bool fullscreen = false;
    const Atom activeProperty = XInternAtom(display, "_NET_ACTIVE_WINDOW", True);
    if (activeProperty != None) {
        const Window active =
            static_cast<Window>(readLongProperty(display, DefaultRootWindow(display),
                                                 activeProperty, XA_WINDOW, 1));
        fullscreen = windowIsFullscreen(display, active);
    }
    XCloseDisplay(display);
    return fullscreen;
}
#elif !defined(Q_OS_WIN) && !defined(Q_OS_MACOS)
bool focusedFullscreenWindowExists() {
    return false;
}
#endif

} // namespace snow_shot::platform
