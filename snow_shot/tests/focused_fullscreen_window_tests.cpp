// Checks the best-effort fullscreen probe that the shortcut manager consults
// before deciding whether to stay quiet.
//
// The assertions that matter are the ones that must not crash: a Wayland
// session, and an X server that offers no EWMH focus information.
#include "snow_shot/platform/focusedfullscreenwindow.h"

#include <QByteArray>
#include <QCoreApplication>
#include <QRectF>
#include <QVector>
#include <QtGlobal>

#include <cstdio>

namespace {
int g_failures = 0;

void require(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++g_failures;
    }
}

using snow_shot::platform::FocusedWindowSnapshot;
using snow_shot::platform::focusedFullscreenWindowExists;
using snow_shot::platform::focusedWindowCoversDisplay;
} // namespace

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);

    // The geometry helper is pure, so it pins down what covering a display means.
    const QVector<QRectF> displays{QRectF(0.0, 0.0, 1920.0, 1080.0)};
    const FocusedWindowSnapshot covering{4242, 0, 1.0, QRectF(0.0, 0.0, 1920.0, 1080.0)};
    require(focusedWindowCoversDisplay(4242, {covering}, displays),
            "a window filling the display must count as covering it");
    const FocusedWindowSnapshot inset{4242, 0, 1.0, QRectF(10.0, 10.0, 1910.0, 1070.0)};
    require(!focusedWindowCoversDisplay(4242, {inset}, displays),
            "a window smaller than the display must not count as covering it");
    const FocusedWindowSnapshot hidden{4242, 1, 1.0, QRectF(0.0, 0.0, 1920.0, 1080.0)};
    require(!focusedWindowCoversDisplay(4242, {hidden}, displays),
            "a window on another layer must not count as covering the display");

    // A Wayland session gives no reliable X11 view of what is fullscreen, so the
    // probe has to decline rather than guess.
    const QByteArray savedWayland = qgetenv("WAYLAND_DISPLAY");
    qputenv("WAYLAND_DISPLAY", "wayland-0");
    require(!focusedFullscreenWindowExists(), "a Wayland session must report no fullscreen window");
    if (savedWayland.isEmpty()) {
        qunsetenv("WAYLAND_DISPLAY");
    } else {
        qputenv("WAYLAND_DISPLAY", savedWayland);
    }

    if (qgetenv("DISPLAY").isEmpty()) {
        std::fprintf(stderr, "skipping the display path: DISPLAY is not set\n");
    } else {
        // Reading a display without a window manager is the case that used to be
        // an unguarded lookup. What is on screen is not ours to assert, so this
        // checks the probe answers consistently and returns at all.
        const bool first = focusedFullscreenWindowExists();
        const bool second = focusedFullscreenWindowExists();
        require(first == second,
                "the probe must answer consistently for an unchanged display");
    }

    if (g_failures != 0) {
        std::fprintf(stderr, "%d focused fullscreen check(s) failed\n", g_failures);
        return 1;
    }
    std::printf("all focused fullscreen checks passed\n");
    return 0;
}
