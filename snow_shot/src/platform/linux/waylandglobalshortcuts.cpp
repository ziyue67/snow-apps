#include "../../presentation/services/globalshortcutbackend_p.h"

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusError>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QUuid>
#include <QDBusObjectPath>
#include <QDBusVariant>
#include <QEventLoop>
#include <QFile>
#include <QGuiApplication>
#include <QHash>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QVariantMap>
#include <Qt>

#include <map>
#include <memory>
#include <mutex>

namespace snow_shot::presentation {

// The portal takes `a(sa{sv})`: an array of (id, properties) structures. The
// element type has to be declared to the marshaller, which an anonymous struct
// streamed by hand cannot do — beginArray(StructureType) left it expecting a
// uint32 array and aborted while writing the structure.
struct PortalShortcut {
    QString id;
    QVariantMap properties;
};

} // namespace snow_shot::presentation

Q_DECLARE_METATYPE(snow_shot::presentation::PortalShortcut)

namespace snow_shot::presentation {

QDBusArgument& operator<<(QDBusArgument& argument, const PortalShortcut& shortcut) {
    argument.beginStructure();
    argument << shortcut.id << shortcut.properties;
    argument.endStructure();
    return argument;
}

// qDBusRegisterMetaType instantiates both directions, so the reading side has to
// exist even though this backend only ever sends the type.
QDBusArgument& operator>>(const QDBusArgument& argument, PortalShortcut& shortcut) {
    argument.beginStructure();
    argument >> shortcut.id >> shortcut.properties;
    argument.endStructure();
    return const_cast<QDBusArgument&>(argument);
}

namespace {

constexpr auto kPortalService = "org.freedesktop.portal.Desktop";
constexpr auto kPortalPath = "/org/freedesktop/portal/desktop";
constexpr auto kShortcutsInterface = "org.freedesktop.portal.GlobalShortcuts";
constexpr auto kRequestInterface = "org.freedesktop.portal.Request";
constexpr auto kSessionInterface = "org.freedesktop.portal.Session";
constexpr auto kHostRegistryInterface = "org.freedesktop.host.portal.Registry";

// A Wayland compositor only brokers global shortcuts for a caller it can
// identify, and it learns that identity here: the portal associates the bus
// connection with the desktop file naming this process. mark-shot registers the
// same way before it opens a shortcut session, and without it the shortcut
// session is created for an unresolved application, so the compositor never
// routes the key press back and the binding silently does nothing.
void registerHostPortalApplication() {
    static std::once_flag once;
    std::call_once(once, [] {
        // Sandboxed builds already carry an identity the portal trusts.
        if (QFile::exists(QStringLiteral("/.flatpak-info")) || qEnvironmentVariableIsSet("SNAP")) {
            return;
        }

        const QString desktopFileName = QGuiApplication::desktopFileName();
        if (desktopFileName.isEmpty()) {
            return;
        }

        QDBusMessage message = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kHostRegistryInterface), QStringLiteral("Register"));
        message << desktopFileName << QVariantMap();

        const QDBusMessage reply = QDBusConnection::sessionBus().call(message, QDBus::Block, 3000);
        if (reply.type() != QDBusMessage::ErrorMessage) {
            return;
        }

        // Older portals predate the interface and report the association as a
        // failure; neither is worth surfacing, the shortcut path still works.
        const QDBusError error(reply);
        if (error.type() == QDBusError::UnknownInterface ||
            error.type() == QDBusError::UnknownMethod) {
            return;
        }
        if (error.name() == QStringLiteral("org.freedesktop.portal.Error.Failed") &&
            error.message().contains(QStringLiteral("Connection already associated"))) {
            return;
        }
        qWarning("global-shortcut portal registration failed: %s",
                 error.message().toUtf8().constData());
    });
}

// Portal results carry `a{sv}`, so a value can arrive either as a plain string
// or wrapped in a variant depending on how the sender typed it.
QString unwrapString(const QVariant& value) {
    if (value.canConvert<QDBusVariant>()) {
        return value.value<QDBusVariant>().variant().toString();
    }
    return value.toString();
}

// The portal names triggers in the XDG shortcuts notation: modifiers in upper
// case, the key itself in lower case ("Ctrl+Shift+A" becomes "CTRL+SHIFT+a").
QString portalTrigger(const QString& portableText) {
    const QStringList parts = portableText.split(QLatin1Char('+'), Qt::SkipEmptyParts);
    if (parts.isEmpty()) {
        return {};
    }
    QStringList trigger;
    for (int index = 0; index + 1 < parts.size(); ++index) {
        const QString modifier = parts.at(index).trimmed();
        if (modifier.compare(QStringLiteral("Ctrl"), Qt::CaseInsensitive) == 0) {
            trigger << QStringLiteral("CTRL");
        } else if (modifier.compare(QStringLiteral("Shift"), Qt::CaseInsensitive) == 0) {
            trigger << QStringLiteral("SHIFT");
        } else if (modifier.compare(QStringLiteral("Alt"), Qt::CaseInsensitive) == 0) {
            trigger << QStringLiteral("ALT");
        } else if (modifier.compare(QStringLiteral("Meta"), Qt::CaseInsensitive) == 0) {
            trigger << QStringLiteral("LOGO");
        }
    }
    trigger << parts.constLast().trimmed().toLower();
    return trigger.join(QLatin1Char('+'));
}

QString shortcutIdFor(int registrationId) {
    return QStringLiteral("action-%1").arg(registrationId);
}

} // namespace

// Global shortcuts through the XDG Desktop Portal.
//
// A Wayland session gives no way to grab keys directly, so the portal brokers
// them: the application creates a session, binds triggers to it, and is told
// when one fires. The compositor may refuse the request outright, and the first
// bind usually prompts the user, so every call here can legitimately fail.
class WaylandGlobalShortcutBackend final : public QObject, public GlobalShortcutBackend {
    Q_OBJECT

  public:
    explicit WaylandGlobalShortcutBackend(QObject* parent = nullptr) : QObject(parent) {}

    ~WaylandGlobalShortcutBackend() override {
        // Every registration opens its own session, so closing only the newest
        // one leaves the rest behind: the portal keeps their shortcuts bound,
        // the next run cannot bind the same keys again, and the key presses go
        // to sessions nobody owns any more.
        // Iterate a copy: closeSession() drops the entry it closes.
        const QStringList sessions = m_sessionPaths;
        for (const QString& session : sessions) {
            closeSession(session);
        }
    }

    // Ending a session is the portal's only way to release the triggers it holds,
    // so both teardown and a re-bind go through here.
    void closeSession(const QString& session) {
        if (session.isEmpty()) {
            return;
        }
        QDBusConnection::sessionBus().disconnect(
            QString::fromLatin1(kPortalService), session, QString::fromLatin1(kShortcutsInterface),
            QStringLiteral("Activated"), this,
            SLOT(onActivated(QDBusObjectPath, QString, qulonglong, QVariantMap)));

        QDBusMessage close = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), session, QString::fromLatin1(kSessionInterface),
            QStringLiteral("Close"));
        QDBusConnection::sessionBus().send(close);
        m_sessionPaths.removeAll(session);
    }

    void setActivationHandler(ActivationHandler handler) override {
        m_handler = std::move(handler);
    }

    GlobalShortcutValidationResult
    validateShortcut(const shortcuts::ShortcutBinding& binding) const override {
        GlobalShortcutValidationResult result;
        result.shortcut = binding.portableText;
        result.binding = binding;
        // The portal accepts a trigger only after the compositor agrees to it,
        // which happens at bind time; validation can only check the shape.
        const QString trigger = portalTrigger(binding.portableText);
        const bool shaped = !trigger.isEmpty() && trigger.contains(QLatin1Char('+'));
        result.supported = shaped;
        result.failureReason = shaped ? GlobalShortcutFailureReason::None
                                      : GlobalShortcutFailureReason::InvalidShortcut;
        return result;
    }

    GlobalShortcutBackendResult
    registerShortcut(int registrationId, const shortcuts::ShortcutBinding& binding) override {
        GlobalShortcutBackendResult result;
        QDBusConnection bus = QDBusConnection::sessionBus();
        if (!bus.isConnected()) {
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }

        const QString trigger = portalTrigger(binding.portableText);
        if (trigger.isEmpty()) {
            result.failureReason = GlobalShortcutFailureReason::InvalidShortcut;
            return result;
        }

        QString error;
        if (!ensureSession(&error)) {
            result.failureReason = GlobalShortcutFailureReason::UnsupportedPlatform;
            return result;
        }

        // One bind per registration keeps the bookkeeping simple; the portal
        // allows binding incrementally on an existing session.
        // The declared type must be known to the marshaller; registering once is
        // enough, and the function-local static keeps it off the hot path.
        static const auto portalShortcutRegistered = qDBusRegisterMetaType<PortalShortcut>();
        Q_UNUSED(portalShortcutRegistered);
        QDBusArgument shortcutList;
        shortcutList.beginArray(qMetaTypeId<PortalShortcut>());
        shortcutList << PortalShortcut{
            shortcutIdFor(registrationId),
            // `preferred_trigger` is the only field the portal reads as the key to
            // grab; `trigger_description` merely labels it for display. Sending the
            // description alone made the compositor accept the bind while binding no
            // trigger at all: it answered ShortcutsChanged with an empty list and
            // never delivered Activated, so every shortcut was silently dead.
            QVariantMap{{QStringLiteral("description"), binding.portableText},
                        {QStringLiteral("preferred_trigger"), trigger}}};
        shortcutList.endArray();

        QDBusMessage bind = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kShortcutsInterface), QStringLiteral("BindShortcuts"));
        // QDBusMessage cannot stream a QDBusArgument directly; a QVariant carries
        // it through the marshaller.
        bind << QDBusObjectPath(m_sessionPath) << QVariant::fromValue(shortcutList) << QString()
             << QVariantMap{{QStringLiteral("handle_token"),
                             QStringLiteral("snow_shot_bind_%1")
                                 .arg(QUuid::createUuid().toString(QUuid::Id128))}};

        QVariantMap results;
        if (!callAndAwait(bind, &results, &error)) {
            result.failureReason = GlobalShortcutFailureReason::SystemError;
            return result;
        }

        m_activationByShortcut.insert(shortcutIdFor(registrationId), registrationId);
        m_sessionByShortcut.insert(shortcutIdFor(registrationId), m_sessionPath);
        result.registered = true;
        result.failureReason = GlobalShortcutFailureReason::None;
        return result;
    }

    void unregisterShortcut(int registrationId) override {
        // The portal has no unbind, so the only way to release a trigger is to
        // end the session that owns it. Dropping just the local mapping left the
        // compositor still holding the key, and because each registration opens
        // its own session the stale one stayed alive: re-binding the same key
        // for a changed shortcut was refused as already taken, which is why
        // editing a shortcut appeared to do nothing.
        const QString id = shortcutIdFor(registrationId);
        for (auto it = m_activationByShortcut.begin(); it != m_activationByShortcut.end();) {
            it = it.key() == id ? m_activationByShortcut.erase(it) : std::next(it);
        }

        const QString sessionPath = m_sessionByShortcut.take(id);
        if (!sessionPath.isEmpty()) {
            closeSession(sessionPath);
        }
    }

  public slots:
    void onRequestResponse(uint response, const QVariantMap& results) {
        m_response = response;
        m_results = results;
        m_awaiting = false;
        if (m_loop != nullptr) {
            m_loop->quit();
        }
    }

    void onActivated(const QDBusObjectPath& session, const QString& shortcutId,
                     qulonglong timestamp, const QVariantMap& options) {
        Q_UNUSED(session);
        Q_UNUSED(timestamp);
        Q_UNUSED(options);
        const auto it = m_activationByShortcut.constFind(shortcutId);
        if (it == m_activationByShortcut.constEnd() || !m_handler) {
            return;
        }
        const int registrationId = it.value();
        ActivationHandler handler = m_handler;
        QMetaObject::invokeMethod(
            this, [handler, registrationId] { handler(registrationId); }, Qt::QueuedConnection);
    }

  private:
    bool ensureSession(QString* error) {
        // Identify this process to the portal before opening a shortcut session,
        // so the compositor attributes the bindings to this application.
        registerHostPortalApplication();

        // Every registration gets its own session. Measured against the portal
        // on this host: within one session only the first BindShortcuts is
        // accepted and later ones come back with response 2, so sharing a
        // session silently dropped every shortcut after the first.
        QDBusMessage create = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kShortcutsInterface), QStringLiteral("CreateSession"));
        // The portal names the session after a token from the options; with an
        // empty map it has no token at all and aborts while initialising the
        // session, which takes the whole portal down and leaves callers with
        // "no reply" instead of an error they could report.
        const QString sessionToken =
            QStringLiteral("snow_shot_%1").arg(QUuid::createUuid().toString(QUuid::Id128));
        create << QVariantMap{{QStringLiteral("handle_token"), sessionToken},
                              {QStringLiteral("session_handle_token"), sessionToken}};

        QVariantMap results;
        if (!callAndAwait(create, &results, error)) {
            return false;
        }
        m_sessionPath = unwrapString(results.value(QStringLiteral("session_handle")));
        if (m_sessionPath.isEmpty()) {
            if (error != nullptr) {
                *error = QStringLiteral("the portal created no shortcut session");
            }
            return false;
        }
        m_sessionPaths.append(m_sessionPath);
        return QDBusConnection::sessionBus().connect(
            QString::fromLatin1(kPortalService), m_sessionPath,
            QString::fromLatin1(kShortcutsInterface), QStringLiteral("Activated"), this,
            SLOT(onActivated(QDBusObjectPath, QString, qulonglong, QVariantMap)));
    }

    // The portal answers a method call with a request handle and reports the
    // outcome later on that object, so the reply is only half the exchange.
    bool callAndAwait(const QDBusMessage& call, QVariantMap* results, QString* error) {
        QDBusConnection bus = QDBusConnection::sessionBus();
        const QDBusMessage reply = bus.call(call);
        if (reply.type() == QDBusMessage::ErrorMessage) {
            if (error != nullptr) {
                *error = reply.errorMessage();
            }
            return false;
        }
        if (reply.arguments().isEmpty()) {
            if (error != nullptr) {
                *error = QStringLiteral("the portal returned no request handle");
            }
            return false;
        }
        const QString requestPath = reply.arguments().constFirst().value<QDBusObjectPath>().path();
        if (requestPath.isEmpty()) {
            if (error != nullptr) {
                *error = QStringLiteral("the portal returned an empty request handle");
            }
            return false;
        }

        QEventLoop loop;
        m_loop = &loop;
        m_awaiting = true;
        m_response = 2;
        const bool subscribed =
            bus.connect(QString::fromLatin1(kPortalService), requestPath,
                        QString::fromLatin1(kRequestInterface), QStringLiteral("Response"), this,
                        SLOT(onRequestResponse(uint, QVariantMap)));
        if (!subscribed) {
            m_loop = nullptr;
            if (error != nullptr) {
                *error = QStringLiteral("could not listen for the portal response");
            }
            return false;
        }
        QTimer::singleShot(30000, &loop, &QEventLoop::quit);
        loop.exec();
        m_loop = nullptr;

        if (m_awaiting) {
            if (error != nullptr) {
                *error = QStringLiteral("the portal did not answer in time");
            }
            return false;
        }
        if (m_response != 0) {
            if (error != nullptr) {
                *error = m_response == 1 ? QStringLiteral("the request was cancelled")
                                         : QStringLiteral("the portal refused the request");
            }
            return false;
        }
        if (results != nullptr) {
            *results = m_results;
        }
        return true;
    }

    ActivationHandler m_handler;
    QString m_sessionPath;
    QStringList m_sessionPaths;
    // Each registration owns a session, so a re-bind has to know which one to end
    // to release the trigger it still holds.
    QHash<QString, QString> m_sessionByShortcut;
    QEventLoop* m_loop = nullptr;
    bool m_awaiting = false;
    uint m_response = 2;
    QVariantMap m_results;
    QMap<QString, int> m_activationByShortcut;
};

std::unique_ptr<GlobalShortcutBackend> createWaylandGlobalShortcutBackend() {
    return std::make_unique<WaylandGlobalShortcutBackend>();
}

} // namespace snow_shot::presentation

#include "waylandglobalshortcuts.moc"
