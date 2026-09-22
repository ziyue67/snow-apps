// Deterministic checks for the XDG Desktop Portal screenshot helper.
//
// The Wayland capture path cannot run in a container without a session, so
// these cover what is verifiable anywhere: the session-type decision, the
// invariant that a failed helper always explains itself, and — when a mock
// portal is running on the private session bus — the full round-trip.
//
// Run the round-trip with a mock portal on a private session bus:
//   dbus-run-session -- bash -c "python3 mock_portal.py & sleep 2; SNOW_SHOT_TEST_PORTAL=1 ./snow-shot-portal-screenshot-tests"
#include "snow_shot/platform/portalscreenshot.h"

#include <QByteArray>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QImage>
#include <QString>

#include <cstdio>

namespace {
int g_failures = 0;

void require(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++g_failures;
    }
}

// The portal is the only way to read a Wayland desktop, and it must not be
// consulted on X11 where the native backend works.
void requirementFollowsTheSession() {
    const QByteArray saved = qgetenv("WAYLAND_DISPLAY");
    qputenv("WAYLAND_DISPLAY", "wayland-0");
    require(snow_shot::platform::portalScreenshotRequired(),
            "a Wayland session must ask the portal for a screenshot");
    qunsetenv("WAYLAND_DISPLAY");
    require(!snow_shot::platform::portalScreenshotRequired(),
            "an X11 session must keep using the native backend");
    if (!saved.isEmpty()) {
        qputenv("WAYLAND_DISPLAY", saved);
    }
}

// Whatever the environment offers, the helper must never hand back nothing
// without saying why.
//
// This only runs where no session bus is reachable: QDBusConnection caches its
// session connection, so forcing a failure here would leave that broken
// connection cached and poison the round-trip below.
void failureIsAlwaysExplained() {
    if (QDBusConnection::sessionBus().isConnected()) {
        std::fprintf(stderr, "skipping the failure check: a session bus is reachable\n");
        return;
    }
    QString error;
    const QImage image = snow_shot::platform::takePortalScreenshot(&error, 2000);
    require(!image.isNull() || !error.isEmpty(),
            "a missing screenshot must come with a reason");
}

// The portal answers with a request handle and reports the result later on that
// object's Response signal, so this exercises the whole exchange at once.
void portalRoundTrip() {
    if (!qEnvironmentVariableIsSet("SNOW_SHOT_TEST_PORTAL")) {
        std::fprintf(stderr, "skipping the portal round-trip: no mock portal is running\n");
        return;
    }
    QString error;
    const QImage image = snow_shot::platform::takePortalScreenshot(&error, 5000);
    if (image.isNull()) {
        std::fprintf(stderr, "portal round-trip reported: %s\n", qPrintable(error));
    }
    require(!image.isNull(), "the image the portal produced must be returned");
    require(image.width() == 1 && image.height() == 1,
            "the portal image dimensions must survive the round-trip");
}
} // namespace

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);
    requirementFollowsTheSession();
    failureIsAlwaysExplained();
    portalRoundTrip();
    if (g_failures != 0) {
        std::fprintf(stderr, "%d portal screenshot check(s) failed\n", g_failures);
        return 1;
    }
    std::printf("all portal screenshot checks passed\n");
    return 0;
}
