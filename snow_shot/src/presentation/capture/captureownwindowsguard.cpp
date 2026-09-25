#include "snow_shot/presentation/captureownwindowsguard.h"

#include <QApplication>
#include <QEventLoop>

CaptureOwnWindowsGuard::CaptureOwnWindowsGuard(bool hideWindows) {
    if (!hideWindows || QApplication::instance() == nullptr) {
        m_restored = true;
        return;
    }

    const QWidgetList topLevelWidgets = QApplication::topLevelWidgets();
    for (QWidget* widget : topLevelWidgets) {
        if (widget == nullptr || !widget->isVisible() ||
            widget->testAttribute(Qt::WA_DontShowOnScreen)) {
            continue;
        }
        m_hiddenWindows.append({widget, widget->windowState()});
        widget->hide();
    }

    if (!m_hiddenWindows.isEmpty()) {
        // The windows have to leave the screen before the compositor composes the frame the
        // capture is about to read, so give the hide a turn to reach the display server.
        QApplication::processEvents(QEventLoop::ExcludeUserInputEvents);
    }
}

CaptureOwnWindowsGuard::~CaptureOwnWindowsGuard() {
    restore();
}

void CaptureOwnWindowsGuard::restore() {
    if (m_restored) {
        return;
    }
    m_restored = true;

    for (const HiddenWindowState& hiddenWindow : std::as_const(m_hiddenWindows)) {
        QWidget* widget = hiddenWindow.widget.data();
        if (widget == nullptr) {
            continue;
        }
        widget->show();
        const Qt::WindowStates restorableStates =
            hiddenWindow.windowStates &
            (Qt::WindowMinimized | Qt::WindowMaximized | Qt::WindowFullScreen);
        if (restorableStates.testFlag(Qt::WindowFullScreen)) {
            widget->showFullScreen();
        } else if (restorableStates.testFlag(Qt::WindowMaximized)) {
            widget->showMaximized();
        } else if (restorableStates.testFlag(Qt::WindowMinimized)) {
            widget->showMinimized();
        }
    }
    if (!m_hiddenWindows.isEmpty() && QApplication::instance() != nullptr) {
        QApplication::processEvents(QEventLoop::ExcludeUserInputEvents);
    }
    m_hiddenWindows.clear();
}
