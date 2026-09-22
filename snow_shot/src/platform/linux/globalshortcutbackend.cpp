#include "../../presentation/services/globalshortcutbackend_p.h"

#include <QKeySequence>
#include <QMetaObject>
#include <QObject>
#include <QString>
#include <Qt>

#include <X11/Xlib.h>
#include <X11/keysym.h>

// Xlib defines None as a macro, which would rewrite
// GlobalShortcutFailureReason::None into an invalid token. Nothing here passes
// None to Xlib, so dropping the macro is safe.
#undef None

#include <sys/select.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <memory>
#include <mutex>
#include <thread>
#include <unordered_map>
#include <utility>

namespace snow_shot::presentation {
namespace {

// Modifiers X reports on a key event that must not take part in matching: the
// lock modifiers vary with the user's keyboard state, not with the shortcut.
constexpr unsigned int kIgnorableModifiers = LockMask | Mod2Mask;

// Grab every combination of the ignorable modifiers so a shortcut keeps firing
// while CapsLock or NumLock is on.
constexpr unsigned int kIgnorableCombinations[] = {
    0,
    LockMask,
    Mod2Mask,
    LockMask | Mod2Mask,
};

// XGrabKey only reaches XWayland: a native Wayland client never routes its keys
// through X, so a grab would appear to register and then never fire. The portal's
// GlobalShortcuts interface is the sanctioned path and is not wired up yet.
bool x11GrabsAreEffective() {
    return qgetenv("WAYLAND_DISPLAY").isEmpty();
}

unsigned int x11ModifierMask(Qt::KeyboardModifiers modifiers) {
    unsigned int mask = 0;
    if (modifiers & Qt::ShiftModifier) {
        mask |= ShiftMask;
    }
    if (modifiers & Qt::ControlModifier) {
        mask |= ControlMask;
    }
    if (modifiers & Qt::AltModifier) {
        mask |= Mod1Mask;
    }
    if (modifiers & Qt::MetaModifier) {
        mask |= Mod4Mask;
    }
    return mask;
}

bool isModifierKey(Qt::Key key) {
    switch (key) {
    case Qt::Key_Shift:
    case Qt::Key_Control:
    case Qt::Key_Meta:
    case Qt::Key_Alt:
    case Qt::Key_AltGr:
    case Qt::Key_CapsLock:
    case Qt::Key_NumLock:
    case Qt::Key_ScrollLock:
        return true;
    default:
        return false;
    }
}

// Letters and digits are the keys a user types with, so a global grab on one of
// them without a modifier would swallow ordinary input. Dedicated keys are fine
// on their own: PrintScreen and the function keys are normal global shortcuts.
bool isTypingKey(Qt::Key key) {
    return (key >= Qt::Key_A && key <= Qt::Key_Z) || (key >= Qt::Key_0 && key <= Qt::Key_9);
}

KeySym keysymForQtKey(Qt::Key key) {
    if (key >= Qt::Key_A && key <= Qt::Key_Z) {
        // Key symbols name the unshifted character, so the letter is lowercase.
        return static_cast<KeySym>(XK_a + (key - Qt::Key_A));
    }
    if (key >= Qt::Key_0 && key <= Qt::Key_9) {
        return static_cast<KeySym>(XK_0 + (key - Qt::Key_0));
    }
    if (key >= Qt::Key_F1 && key <= Qt::Key_F35) {
        return static_cast<KeySym>(XK_F1 + (key - Qt::Key_F1));
    }
    switch (key) {
    case Qt::Key_Space:
        return XK_space;
    case Qt::Key_Tab:
        return XK_Tab;
    case Qt::Key_Backtab:
        return XK_ISO_Left_Tab;
    case Qt::Key_Return:
    case Qt::Key_Enter:
        return XK_Return;
    case Qt::Key_Escape:
        return XK_Escape;
    case Qt::Key_Backspace:
        return XK_BackSpace;
    case Qt::Key_Delete:
        return XK_Delete;
    case Qt::Key_Insert:
        return XK_Insert;
    case Qt::Key_Home:
        return XK_Home;
    case Qt::Key_End:
        return XK_End;
    case Qt::Key_PageUp:
        return XK_Page_Up;
    case Qt::Key_PageDown:
        return XK_Page_Down;
    case Qt::Key_Left:
        return XK_Left;
    case Qt::Key_Right:
        return XK_Right;
    case Qt::Key_Up:
        return XK_Up;
    case Qt::Key_Down:
        return XK_Down;
    case Qt::Key_Print:
        return XK_Print;
    case Qt::Key_Pause:
        return XK_Pause;
    case Qt::Key_Menu:
        return XK_Menu;
    case Qt::Key_Comma:
        return XK_comma;
    case Qt::Key_Period:
        return XK_period;
    case Qt::Key_Slash:
        return XK_slash;
    case Qt::Key_Semicolon:
        return XK_semicolon;
    case Qt::Key_Apostrophe:
        return XK_apostrophe;
    case Qt::Key_BracketLeft:
        return XK_bracketleft;
    case Qt::Key_BracketRight:
        return XK_bracketright;
    case Qt::Key_Backslash:
        return XK_backslash;
    case Qt::Key_Minus:
        return XK_minus;
    case Qt::Key_Equal:
        return XK_equal;
    case Qt::Key_QuoteLeft:
        return XK_grave;
    default:
        return NoSymbol;
    }
}

// XGrabKey reports nothing when another client already holds the combination:
// the server raises BadAccess instead. Catch it with a temporary handler so the
// caller learns the shortcut is taken rather than believing it registered.
std::atomic<bool> g_grabRejected{false};

int grabErrorHandler(Display*, XErrorEvent* event) {
    if (event != nullptr && event->error_code == BadAccess) {
        g_grabRejected.store(true);
    }
    return 0;
}

struct Registration {
    KeyCode keyCode = 0;
    unsigned int modifiers = 0;
};

} // namespace

// X11 implementation of the global shortcut backend.
//
// Keys are grabbed on the root window of a private display connection, and a
// worker thread turns the resulting KeyPress events back into activations. The
// connection is separate from Qt's own so the grab cannot interfere with the
// application's event delivery.
class LinuxGlobalShortcutBackend final : public QObject, public GlobalShortcutBackend {
  public:
    LinuxGlobalShortcutBackend() {
        static std::once_flag initFlag;
        std::call_once(initFlag, [] { XInitThreads(); });

        m_display = XOpenDisplay(nullptr);
        if (m_display == nullptr) {
            return;
        }
        m_root = DefaultRootWindow(m_display);
        // KeyPress events reach us only if the root window reports them.
        XSelectInput(m_display, m_root, KeyPressMask);

        if (::pipe(m_wakeup) != 0) {
            XCloseDisplay(m_display);
            m_display = nullptr;
            return;
        }
        m_running.store(true);
        m_thread = std::thread([this] { eventLoop(); });
    }

    ~LinuxGlobalShortcutBackend() override {
        if (m_display == nullptr) {
            return;
        }
        m_running.store(false);
        const char wake = 'w';
        [[maybe_unused]] const ssize_t written = ::write(m_wakeup[1], &wake, 1);
        if (m_thread.joinable()) {
            m_thread.join();
        }
        ::close(m_wakeup[0]);
        ::close(m_wakeup[1]);
        // Release every grab before tearing the connection down.
        std::lock_guard<std::mutex> lock(m_mutex);
        for (const auto& [id, registration] : m_registrations) {
            ungrab(m_display, m_root, registration);
        }
        m_registrations.clear();
        XCloseDisplay(m_display);
        m_display = nullptr;
    }

    void setActivationHandler(ActivationHandler handler) override {
        std::lock_guard<std::mutex> lock(m_mutex);
        m_handler = std::move(handler);
    }

    GlobalShortcutValidationResult
    validateShortcut(const shortcuts::ShortcutBinding& binding) const override {
        GlobalShortcutValidationResult result;
        result.shortcut = binding.portableText;
        result.binding = binding;
        if (!x11GrabsAreEffective()) {
            result.supported = false;
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }

        const ParsedShortcut parsed = parse(binding.portableText);
        if (parsed.valid) {
            result.supported = true;
            result.failureReason = GlobalShortcutFailureReason::None;
            return result;
        }
        // Distinguish a sequence the window system cannot express from one that
        // is malformed, so the settings UI can explain the difference.
        result.supported = false;
        result.failureReason = parsed.parsed ? GlobalShortcutFailureReason::UnsupportedPlatform
                                             : GlobalShortcutFailureReason::InvalidShortcut;
        return result;
    }

    GlobalShortcutBackendResult registerShortcut(int registrationId,
                                                 const shortcuts::ShortcutBinding& binding) override {
        GlobalShortcutBackendResult result;
        if (!x11GrabsAreEffective()) {
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }
        if (m_display == nullptr) {
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }

        const ParsedShortcut parsed = parse(binding.portableText);
        if (!parsed.valid) {
            result.failureReason = parsed.parsed ? GlobalShortcutFailureReason::UnsupportedPlatform
                                                 : GlobalShortcutFailureReason::InvalidShortcut;
            return result;
        }
        const KeyCode keyCode = XKeysymToKeycode(m_display, parsed.keySym);
        if (keyCode == 0) {
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }

        // Serialised because the error handler below is process wide.
        std::lock_guard<std::mutex> lock(m_mutex);
        unregisterLocked(registrationId);

        Registration registration;
        registration.keyCode = keyCode;
        registration.modifiers = parsed.modifiers;

        g_grabRejected.store(false);
        XErrorHandler previous = XSetErrorHandler(grabErrorHandler);
        for (const unsigned int combination : kIgnorableCombinations) {
            XGrabKey(m_display, static_cast<int>(keyCode), registration.modifiers | combination,
                     m_root, True, GrabModeAsync, GrabModeAsync);
        }
        XSync(m_display, False);
        XSetErrorHandler(previous);

        if (g_grabRejected.load()) {
            ungrab(m_display, m_root, registration);
            XSync(m_display, False);
            result.failureReason = GlobalShortcutFailureReason::AlreadyInUse;
            return result;
        }

        m_registrations.emplace(registrationId, registration);
        result.registered = true;
        result.failureReason = GlobalShortcutFailureReason::None;
        return result;
    }

    void unregisterShortcut(int registrationId) override {
        if (m_display == nullptr) {
            return;
        }
        std::lock_guard<std::mutex> lock(m_mutex);
        unregisterLocked(registrationId);
    }

  private:
    struct ParsedShortcut {
        // The text contained a usable key sequence, whether or not X can express it.
        bool parsed = false;
        bool valid = false;
        KeySym keySym = NoSymbol;
        unsigned int modifiers = 0;
    };

    static ParsedShortcut parse(const QString& portableText) {
        ParsedShortcut parsed;
        const QKeySequence sequence =
            QKeySequence::fromString(portableText, QKeySequence::PortableText);
        if (sequence.isEmpty()) {
            return parsed;
        }
        const QKeyCombination combination = sequence[0];
        const Qt::Key key = combination.key();
        if (key == Qt::Key_unknown || isModifierKey(key)) {
            return parsed;
        }
        parsed.parsed = true;

        const unsigned int modifiers = x11ModifierMask(combination.keyboardModifiers());
        if (isTypingKey(key) && modifiers == 0) {
            // Reported as parsed so the reason is "unsupported" rather than
            // "malformed": a bare letter is understood, just not grabbable.
            return parsed;
        }

        const KeySym keySym = keysymForQtKey(key);
        if (keySym == NoSymbol) {
            return parsed;
        }
        parsed.keySym = keySym;
        parsed.modifiers = modifiers;
        parsed.valid = true;
        return parsed;
    }

    static void ungrab(Display* display, Window root, const Registration& registration) {
        for (const unsigned int combination : kIgnorableCombinations) {
            XUngrabKey(display, static_cast<int>(registration.keyCode),
                       registration.modifiers | combination, root);
        }
    }

    void unregisterLocked(int registrationId) {
        const auto it = m_registrations.find(registrationId);
        if (it == m_registrations.end()) {
            return;
        }
        ungrab(m_display, m_root, it->second);
        XSync(m_display, False);
        m_registrations.erase(it);
    }

    void eventLoop() {
        const int xfd = ConnectionNumber(m_display);
        const int wakefd = m_wakeup[0];
        while (m_running.load()) {
            fd_set readSet;
            FD_ZERO(&readSet);
            FD_SET(xfd, &readSet);
            FD_SET(wakefd, &readSet);
            timeval timeout{};
            timeout.tv_usec = 100000;
            const int ready =
                ::select(std::max(xfd, wakefd) + 1, &readSet, nullptr, nullptr, &timeout);
            if (ready <= 0) {
                continue;
            }
            if (FD_ISSET(wakefd, &readSet)) {
                char drain[64];
                while (::read(wakefd, drain, sizeof(drain)) > 0) {
                }
            }
            while (XPending(m_display) > 0) {
                XEvent event{};
                XNextEvent(m_display, &event);
                if (event.type == KeyPress) {
                    handleKeyPress(event.xkey);
                }
            }
        }
    }

    void handleKeyPress(const XKeyEvent& event) {
        ActivationHandler handler;
        int registrationId = -1;
        {
            std::lock_guard<std::mutex> lock(m_mutex);
            const unsigned int modifiers =
                static_cast<unsigned int>(event.state) & ~kIgnorableModifiers;
            for (const auto& [id, registration] : m_registrations) {
                if (registration.keyCode == event.keycode && registration.modifiers == modifiers) {
                    registrationId = id;
                    handler = m_handler;
                    break;
                }
            }
        }
        if (registrationId < 0 || !handler) {
            return;
        }
        // The manager reacts on the main thread, so hand the activation over
        // rather than calling into it from this one.
        QMetaObject::invokeMethod(
            this, [handler, registrationId] { handler(registrationId); }, Qt::QueuedConnection);
    }

    Display* m_display = nullptr;
    Window m_root = 0;
    int m_wakeup[2] = {-1, -1};
    std::atomic<bool> m_running{false};
    std::thread m_thread;
    mutable std::mutex m_mutex;
    std::unordered_map<int, Registration> m_registrations;
    ActivationHandler m_handler;
};

std::unique_ptr<GlobalShortcutBackend> createLinuxGlobalShortcutBackend() {
    return std::make_unique<LinuxGlobalShortcutBackend>();
}

} // namespace snow_shot::presentation
