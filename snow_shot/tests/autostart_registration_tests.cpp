// Checks the XDG autostart entry that the settings toggle drives.
//
// The entry is a file under XDG_CONFIG_HOME, so pointing that at a temporary
// directory makes the whole thing deterministic and needs no desktop session.
#include "snow_shot/platform/windows/autostartregistration.h"

#include <QByteArray>
#include <QCoreApplication>
#include <QString>
#include <QTemporaryDir>
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

using snow_shot::platform::windows::AutoStartRegistration;
using snow_shot::platform::windows::AutoStartRegistrationSnapshot;
} // namespace

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);

    QTemporaryDir configHome;
    if (!configHome.isValid()) {
        std::fprintf(stderr, "FAIL: no temporary directory to use as XDG_CONFIG_HOME\n");
        return 1;
    }
    qputenv("XDG_CONFIG_HOME", configHome.path().toUtf8());

    require(AutoStartRegistration::isSupported(),
            "an XDG autostart entry is available on Linux");
    require(!AutoStartRegistration::expectedCommand().isEmpty(),
            "the entry needs a command to run");
    require(!AutoStartRegistration::matchesExpectedCommand(),
            "nothing is registered before the entry is written");

    QString error;
    const AutoStartRegistrationSnapshot before = AutoStartRegistration::snapshot();
    require(before.valid && !before.exists,
            "an absent entry reads as valid and not present");

    // Enabling writes the entry, and it names the command the application expects.
    require(AutoStartRegistration::setEnabled(true, &error), "enabling must write the entry");
    require(error.isEmpty(), "enabling must not report an error");
    require(AutoStartRegistration::matchesExpectedCommand(),
            "the written entry must carry the expected command");
    const AutoStartRegistrationSnapshot enabled = AutoStartRegistration::snapshot();
    require(enabled.valid && enabled.exists && !enabled.nativeData.isEmpty(),
            "the written entry must be readable back");

    // Disabling removes it again.
    require(AutoStartRegistration::setEnabled(false, &error), "disabling must remove the entry");
    require(!AutoStartRegistration::matchesExpectedCommand(),
            "a removed entry must stop matching");
    const AutoStartRegistrationSnapshot disabled = AutoStartRegistration::snapshot();
    require(disabled.valid && !disabled.exists, "a removed entry reads as absent");

    // Restoring a captured snapshot puts the entry back, and restoring the
    // original absent state takes it away again.
    require(AutoStartRegistration::restore(enabled, &error),
            "restoring the captured entry must succeed");
    require(AutoStartRegistration::matchesExpectedCommand(), "a restored entry must match again");
    require(AutoStartRegistration::restore(before, &error),
            "restoring the original absent state must succeed");
    require(!AutoStartRegistration::matchesExpectedCommand(),
            "restoring the absent state must remove the entry again");

    if (g_failures != 0) {
        std::fprintf(stderr, "%d autostart check(s) failed\n", g_failures);
        return 1;
    }
    std::printf("all autostart registration checks passed\n");
    return 0;
}
