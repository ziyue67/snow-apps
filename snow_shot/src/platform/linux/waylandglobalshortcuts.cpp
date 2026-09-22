#include "../../presentation/services/globalshortcutbackend_p.h"

#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusObjectPath>
#include <QDBusVariant>
#include <QEventLoop>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QVariantMap>
#include <Qt>

#include <map>
#include <memory>

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
        if (!m_sessionPath.isEmpty()) {
            QDBusMessage close = QDBusMessage::createMethodCall(
                QString::fromLatin1(kPortalService), m_sessionPath,
                QString::fromLatin1(kSessionInterface), QStringLiteral("Close"));
            QDBusConnection::sessionBus().send(close);
        }
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
        result.failureReason =
            shaped ? GlobalShortcutFailureReason::None : GlobalShortcutFailureReason::InvalidShortcut;
        return result;
    }

    GlobalShortcutBackendResult registerShortcut(int registrationId,
                                                 const shortcuts::ShortcutBinding& binding) override {
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
        static const auto portalShortcutRegistered =
            qDBusRegisterMetaType<PortalShortcut>();
        Q_UNUSED(portalShortcutRegistered);
        QDBusArgument shortcutList;
        shortcutList.beginArray(qMetaTypeId<PortalShortcut>());
        shortcutList << PortalShortcut{
            shortcutIdFor(registrationId),
            QVariantMap{{QStringLiteral("description"), binding.portableText},
                        {QStringLiteral("trigger_description"), trigger}}};
        shortcutList.endArray();

        QDBusMessage bind = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kShortcutsInterface), QStringLiteral("BindShortcuts"));
        // QDBusMessage cannot stream a QDBusArgument directly; a QVariant carries
        // it through the marshaller.
        bind << QDBusObjectPath(m_sessionPath) << QVariant::fromValue(shortcutList)
             << QString() << QVariantMap();

        QVariantMap results;
        if (!callAndAwait(bind, &results, &error)) {
            result.failureReason = GlobalShortcutFailureReason::SystemError;
            return result;
        }

        m_activationByShortcut.insert(shortcutIdFor(registrationId), registrationId);
        result.registered = true;
        result.failureReason = GlobalShortcutFailureReason::None;
        return result;
    }

    void unregisterShortcut(int registrationId) override {
        // The portal has no unbind: the trigger stays bound to this session
        // until it ends, so only the local mapping is dropped.
        const QString id = shortcutIdFor(registrationId);
        for (auto it = m_activationByShortcut.begin(); it != m_activationByShortcut.end();) {
            it = it.key() == id ? m_activationByShortcut.erase(it) : std::next(it);
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

    void onActivated(const QDBusObjectPath& session, const QString& shortcutId, qulonglong timestamp,
                     const QVariantMap& options) {
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
        if (!m_sessionPath.isEmpty()) {
            return true;
        }
        QDBusMessage create = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kShortcutsInterface), QStringLiteral("CreateSession"));
        create << QVariantMap();

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
        const bool subscribed = bus.connect(
            QString::fromLatin1(kPortalService), requestPath,
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
