// Checks that the X11 global shortcut backend does not claim success on a
// Wayland session.
//
// The platform factory now routes a Wayland session to the portal backend, so
// this constructs the X11 backend directly: the guard still matters as defence
// in depth, and it is the only thing standing between a Wayland session and a
// grab that registers and then never fires.
//
// XGrabKey only reaches XWayland: a native Wayland client never routes its keys
// through X, so a grab registers and then never fires. The backend has to report
// the platform as unsupported instead of letting the settings UI show a shortcut
// that cannot work.
//
// Validation parses the binding without touching X, so this runs without a
// display and still tells the guard apart from a missing X server.
#include "../src/presentation/services/globalshortcutbackend_p.h"

#include <QByteArray>
#include <QCoreApplication>
#include <QEventLoop>
#include <QSet>
#include <QString>
#include <QTimer>

#include <cstdio>
#include <memory>

namespace {
int g_failures = 0;

void require(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        ++g_failures;
    }
}

using snow_shot::presentation::GlobalShortcutBackend;
using snow_shot::presentation::GlobalShortcutFailureReason;
using snow_shot::shortcuts::ShortcutBinding;

// With a mock portal on the session bus the whole exchange runs for real: the
// session is created, the shortcut is bound, and the activation comes back with
// the id it was registered under. This is opt-in because it needs that mock.
void portalRoundTrip() {
    if (!qEnvironmentVariableIsSet("SNOW_SHOT_TEST_PORTAL")) {
        std::fprintf(stderr, "skipping the portal round-trip: no mock portal is running\n");
        return;
    }
    qputenv("WAYLAND_DISPLAY", "wayland-0");
    const std::unique_ptr<GlobalShortcutBackend> backend =
        snow_shot::presentation::createPlatformGlobalShortcutBackend();

    int activated = -1;
    backend->setActivationHandler([&activated](int registrationId) { activated = registrationId; });

    const ShortcutBinding binding(QStringLiteral("Ctrl+Shift+A"));
    const auto registered = backend->registerShortcut(1, binding);
    require(registered.registered, "the portal must accept a bound shortcut");

    // The activation arrives on the event loop, so pump it until it lands.
    QEventLoop loop;
    QTimer watchdog;
    watchdog.setSingleShot(true);
    QObject::connect(&watchdog, &QTimer::timeout, &loop, &QEventLoop::quit);
    watchdog.start(5000);
    QTimer poll;
    QObject::connect(&poll, &QTimer::timeout, [&activated, &loop] {
        if (activated >= 0) {
            loop.quit();
        }
    });
    poll.start(50);
    loop.exec();

    require(activated == 1, "the activation must carry the registration id it was bound with");
    qunsetenv("WAYLAND_DISPLAY");
}

// Every registration opens its own portal session, and the portal reports an
// activation against the session it was bound on. Accepting only the newest
// session therefore dropped every shortcut bound before it: with two shortcuts
// registered the first one went silent, which is also what made changing one
// shortcut stop the others from firing.
void portalKeepsOlderSessionsAlive() {
    if (!qEnvironmentVariableIsSet("SNOW_SHOT_TEST_PORTAL")) {
        std::fprintf(stderr, "skipping the multi-session check: no mock portal is running\n");
        return;
    }
    qputenv("WAYLAND_DISPLAY", "wayland-0");
    const std::unique_ptr<GlobalShortcutBackend> backend =
        snow_shot::presentation::createPlatformGlobalShortcutBackend();

    QSet<int> activated;
    backend->setActivationHandler(
        [&activated](int registrationId) { activated.insert(registrationId); });

    const auto first =
        backend->registerShortcut(1, ShortcutBinding(QStringLiteral("Ctrl+Shift+A")));
    const auto second =
        backend->registerShortcut(2, ShortcutBinding(QStringLiteral("Ctrl+Shift+B")));
    require(first.registered && second.registered, "the portal must accept both bound shortcuts");

    // The mock holds activations back until a second session exists, so both
    // arrive only if an older session is still accepted.
    QEventLoop loop;
    QTimer watchdog;
    watchdog.setSingleShot(true);
    QObject::connect(&watchdog, &QTimer::timeout, &loop, &QEventLoop::quit);
    watchdog.start(8000);
    QTimer poll;
    QObject::connect(&poll, &QTimer::timeout, [&activated, &loop] {
        if (activated.size() >= 2) {
            loop.quit();
        }
    });
    poll.start(50);
    loop.exec();

    require(activated.contains(1), "the first shortcut's activation must still be delivered");
    require(activated.contains(2), "the second shortcut's activation must be delivered");
    qunsetenv("WAYLAND_DISPLAY");
}
} // namespace

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);
    const QByteArray saved = qgetenv("WAYLAND_DISPLAY");
    const ShortcutBinding binding(QStringLiteral("Ctrl+Shift+A"));

    qunsetenv("WAYLAND_DISPLAY");
    {
        const std::unique_ptr<GlobalShortcutBackend> backend =
            snow_shot::presentation::createLinuxGlobalShortcutBackend();
        const auto validation = backend->validateShortcut(binding);
        require(validation.supported,
                "an X11 session must accept an ordinary shortcut");
    }

    qputenv("WAYLAND_DISPLAY", "wayland-0");
    {
        const std::unique_ptr<GlobalShortcutBackend> backend =
            snow_shot::presentation::createLinuxGlobalShortcutBackend();
        const auto validation = backend->validateShortcut(binding);
        require(!validation.supported,
                "a Wayland session must not call the shortcut usable");
        require(validation.failureReason == GlobalShortcutFailureReason::UnsupportedPlatform,
                "a Wayland session must name the platform as the reason");

        const auto registered = backend->registerShortcut(1, binding);
        require(!registered.registered,
                "a Wayland session must not report the shortcut as registered");
        require(registered.failureReason == GlobalShortcutFailureReason::UnsupportedPlatform,
                "a refused registration must name the platform");
    }

    portalRoundTrip();
    portalKeepsOlderSessionsAlive();

    if (saved.isEmpty()) {
        qunsetenv("WAYLAND_DISPLAY");
    } else {
        qputenv("WAYLAND_DISPLAY", saved);
    }

    if (g_failures != 0) {
        std::fprintf(stderr, "%d shortcut backend check(s) failed\n", g_failures);
        return 1;
    }
    std::printf("all global shortcut backend checks passed\n");
    return 0;
}
