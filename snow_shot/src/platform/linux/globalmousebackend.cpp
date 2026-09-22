// X11 global mouse backend.
//
// Watches pointer buttons through XInput2 raw events, so the gesture is seen
// wherever the press lands, and hands each one to the shared gesture layer,
// which decides whether the combination the user configured was used.
//
// Raw events deliberately carry no screen coordinates, so the position is read
// with XQueryPointer when an event arrives.
#include "snow_shot/presentation/globalmousemanager.h"

#include <QAbstractNativeEventFilter>
#include <QCoreApplication>
#include <QGuiApplication>
#include <QPointF>
#include <QString>

#include <X11/Xlib.h>
#include <X11/extensions/XInput2.h>
#include <xcb/xcb.h>

// Xlib defines Status as a macro, which would rewrite the permission enum below.
#undef Status

namespace snow_shot::presentation {
namespace {

Qt::MouseButton buttonFromDetail(int detail) {
    switch (detail) {
    case 1:
        return Qt::LeftButton;
    case 2:
        return Qt::MiddleButton;
    case 3:
        return Qt::RightButton;
    default:
        // X11 reports the extra buttons as 8/9 and above.
        return static_cast<Qt::MouseButton>(static_cast<int>(Qt::LeftButton) + detail);
    }
}

GlobalMouseInput::Kind kindFromRawType(int rawType) {
    if (rawType == XI_RawButtonPress) {
        return GlobalMouseInput::Kind::Press;
    }
    if (rawType == XI_RawButtonRelease) {
        return GlobalMouseInput::Kind::Release;
    }
    return GlobalMouseInput::Kind::Move;
}

} // namespace

class LinuxGlobalMouseBackend final : public QObject,
                                      public GlobalMouseBackend,
                                      public QAbstractNativeEventFilter {
  public:
    ~LinuxGlobalMouseBackend() override { stop(); }

    void start(Handler handler, FailureHandler failure) override {
        m_handler = std::move(handler);
        m_failure = std::move(failure);
        Display* display = x11Display();
        if (display == nullptr) {
            reportFailure(kNoDisplay);
            return;
        }
        m_display = display;

        int opcode = 0;
        int event = 0;
        int error = 0;
        if (!XQueryExtension(display, "XInputExtension", &opcode, &event, &error)) {
            reportFailure(kNoXInput);
            return;
        }
        m_xinputOpcode = opcode;

        unsigned char mask[XIMaskLen(XI_LASTEVENT)] = {};
        XIEventMask eventMask{};
        eventMask.deviceid = XIAllMasterDevices;
        eventMask.mask_len = sizeof(mask);
        eventMask.mask = mask;
        XISetMask(mask, XI_RawButtonPress);
        XISetMask(mask, XI_RawButtonRelease);
        XISetMask(mask, XI_RawMotion);
        if (XISelectEvents(display, DefaultRootWindow(display), &eventMask, 1) != Success) {
            reportFailure(kNoRawEvents);
            return;
        }
        XFlush(display);

        QCoreApplication::instance()->installNativeEventFilter(this);
        m_running = true;
    }

    void stop() override {
        if (m_running) {
            QCoreApplication::instance()->removeNativeEventFilter(this);
            m_running = false;
        }
        m_display = nullptr;
        m_handler = nullptr;
        m_failure = nullptr;
    }

    void configure(const GlobalMouseConfiguration& configuration) override {
        m_configuration = configuration;
    }

    void cancel(quint64 id) override {
        Q_UNUSED(id)
        m_gesture.reset();
    }

    void beginButtonDrag(settings::SettingsGlobalMouseAction action) override {
        const GlobalMouseInputResult result = m_gesture.beginButtonDrag(action, pointerPosition());
        if (result.event && m_handler) {
            m_handler(*result.event);
        }
    }

    // X11 has no permission gate for reading the pointer, so the backend is
    // always usable once the extension answered.
    GlobalMousePermissionState permissionState() const override {
        return {GlobalMousePermissionState::Status::Ready, true, true, true};
    }

    bool nativeEventFilter(const QByteArray& eventType, void* message,
                           qintptr* result) override {
        Q_UNUSED(result)
        if (!m_running || eventType != QByteArrayLiteral("xcb_generic_event_t")) {
            return false;
        }
        auto* event = static_cast<xcb_generic_event_t*>(message);
        const int type = event->response_type & 0x7f;
        if (type != m_xinputOpcode + 0) {
            return false;
        }
        // The extension opcode and the event type live in the generic event
        // header that follows the plain xcb event header.
        const auto* generic = reinterpret_cast<const xcb_ge_generic_event_t*>(event);
        if (generic->extension != static_cast<uint8_t>(m_xinputOpcode)) {
            return false;
        }
        const int rawType = generic->event_type;
        if (rawType != XI_RawButtonPress && rawType != XI_RawButtonRelease &&
            rawType != XI_RawMotion) {
            return false;
        }
        deliver(rawType, event);
        return false;
    }

  private:
    void deliver(int rawType, xcb_generic_event_t* event) {
        // The raw event body starts after the 32-byte xcb_ge_event_t header.
        const auto* detail = reinterpret_cast<const uint32_t*>(
            reinterpret_cast<const char*>(event) + sizeof(xcb_ge_generic_event_t));

        GlobalMouseInput input;
        input.kind = kindFromRawType(rawType);
        input.position = pointerPosition();
        if (rawType != XI_RawMotion) {
            input.button = buttonFromDetail(static_cast<int>(*detail));
        }

        const GlobalMouseInputResult result = m_gesture.handle(input, m_configuration);
        if (result.event && m_handler) {
            m_handler(*result.event);
        }
    }

    QPointF pointerPosition() const {
        if (m_display == nullptr) {
            return {};
        }
        Window root = DefaultRootWindow(m_display);
        Window rootReturn = 0;
        Window childReturn = 0;
        int rootX = 0;
        int rootY = 0;
        int windowX = 0;
        int windowY = 0;
        unsigned int mask = 0;
        if (XQueryPointer(m_display, root, &rootReturn, &childReturn, &rootX, &rootY, &windowX,
                          &windowY, &mask) == False) {
            return {};
        }
        return QPointF(rootX, rootY);
    }

    static Display* x11Display() {
        if (auto* x11 = qGuiApp->nativeInterface<QNativeInterface::QX11Application>()) {
            return x11->display();
        }
        return nullptr;
    }

    void reportFailure(quint32 code) {
        if (m_failure) {
            m_failure(code);
        }
    }

    static constexpr quint32 kNoDisplay = 1;
    static constexpr quint32 kNoXInput = 2;
    static constexpr quint32 kNoRawEvents = 3;

    Display* m_display = nullptr;
    int m_xinputOpcode = 0;
    bool m_running = false;
    Handler m_handler;
    FailureHandler m_failure;
    GlobalMouseConfiguration m_configuration;
    GlobalMouseGesture m_gesture;
};

std::unique_ptr<GlobalMouseBackend> createLinuxGlobalMouseBackend() {
    return std::make_unique<LinuxGlobalMouseBackend>();
}

} // namespace snow_shot::presentation
