#ifndef SNOW_SHOT_PRESENTATION_CAPTUREOWNWINDOWSGUARD_H
#define SNOW_SHOT_PRESENTATION_CAPTUREOWNWINDOWSGUARD_H

#include <QPointer>
#include <QVector>
#include <QWidget>

// Hides this application's visible top-level windows for the duration of one capture and
// shows them again afterwards.
//
// The compositor hands over whatever is on screen, so a window this process owns lands in
// the frozen frame: a capture started from the tray menu, a floating palette or a pinned
// window contains that window and the screenshot is not the desktop the user asked for.
// The reference implementation hides its own windows during capture for the same reason,
// and does so by default.
class CaptureOwnWindowsGuard final {
  public:
    explicit CaptureOwnWindowsGuard(bool hideWindows = true);
    ~CaptureOwnWindowsGuard();

    // Shows the windows that were visible when the guard was created.
    void restore();

    // True while the guard holds at least one window off the screen. A capture that hid
    // something has to let the compositor compose a frame without those windows before it
    // reads the screen.
    [[nodiscard]] bool hidAnyWindow() const {
        return !m_hiddenWindows.isEmpty();
    }

    CaptureOwnWindowsGuard(const CaptureOwnWindowsGuard&) = delete;
    CaptureOwnWindowsGuard& operator=(const CaptureOwnWindowsGuard&) = delete;

  private:
    struct HiddenWindowState {
        QPointer<QWidget> widget;
        Qt::WindowStates windowStates = Qt::WindowNoState;
    };

    QVector<HiddenWindowState> m_hiddenWindows;
    bool m_restored = false;
};

#endif // SNOW_SHOT_PRESENTATION_CAPTUREOWNWINDOWSGUARD_H
