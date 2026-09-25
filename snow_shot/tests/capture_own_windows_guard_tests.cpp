// Covers the own-window policy the reference implementation applies to every capture: the
// windows this process owns leave the screen while the capture reads it, so a screenshot
// started from the tray menu or another own window cannot contain that window.
#include "snow_shot/presentation/captureownwindowsguard.h"

#include <QApplication>
#include <QLabel>

#include <cstdlib>
#include <iostream>

namespace {
void require(bool value, const char* message) {
    if (!value) {
        std::cerr << message << '\n';
        std::exit(1);
    }
}

void visibleOwnWindowsLeaveTheScreenForTheCapture() {
    QLabel menu;
    menu.setWindowTitle(QStringLiteral("menu"));
    menu.resize(120, 40);
    menu.show();
    QLabel hidden;
    hidden.setWindowTitle(QStringLiteral("hidden"));
    hidden.resize(120, 40);
    require(menu.isVisible() && !hidden.isVisible(), "the fixture must start visible and hidden");

    {
        CaptureOwnWindowsGuard guard;
        require(!menu.isVisible(), "a visible own window must leave the screen for the capture");
        require(!hidden.isVisible(), "an already hidden window stays hidden");
    }
    require(menu.isVisible(), "the window the capture hid must come back");
    require(!hidden.isVisible(), "a window that was hidden before the capture stays hidden");
}

void everyVisibleTopLevelWindowIsHidden() {
    QLabel first;
    first.resize(80, 30);
    first.show();
    QLabel second;
    second.resize(80, 30);
    second.show();

    const auto visibleCount = []() {
        int count = 0;
        for (QWidget* widget : QApplication::topLevelWidgets()) {
            if (widget != nullptr && widget->isVisible()) {
                ++count;
            }
        }
        return count;
    };
    require(visibleCount() >= 2, "the fixture must show two own windows");

    CaptureOwnWindowsGuard guard;
    require(visibleCount() == 0, "the capture must not see any own top-level window");
    guard.restore();
    require(visibleCount() >= 2, "restoring shows the own windows again");
}

void aDisabledGuardTouchesNothing() {
    QLabel window;
    window.resize(80, 30);
    window.show();

    CaptureOwnWindowsGuard guard(false);
    require(window.isVisible(), "a disabled guard leaves the window on screen");
}
} // namespace

int main(int argc, char** argv) {
    QApplication app(argc, argv);
    visibleOwnWindowsLeaveTheScreenForTheCapture();
    everyVisibleTopLevelWindowIsHidden();
    aDisabledGuardTouchesNothing();
    std::cout << "capture own windows tests passed\n";
    return 0;
}
