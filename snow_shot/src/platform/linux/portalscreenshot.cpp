#include "snow_shot/platform/portalscreenshot.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusVariant>
#include <QEventLoop>
#include <QObject>
#include <QTimer>
#include <QUrl>
#include <QUuid>
#include <QVariantMap>

namespace snow_shot::platform {
namespace {

constexpr auto kPortalService = "org.freedesktop.portal.Desktop";
constexpr auto kPortalPath = "/org/freedesktop/portal/desktop";
constexpr auto kScreenshotInterface = "org.freedesktop.portal.Screenshot";
constexpr auto kRequestInterface = "org.freedesktop.portal.Request";

// Portal results carry `a{sv}`, so a value can arrive either as a plain string
// or wrapped in a variant depending on how the sender typed it.
QString stringFromVariant(const QVariant& value) {
    if (value.canConvert<QDBusVariant>()) {
        return value.value<QDBusVariant>().variant().toString();
    }
    return value.toString();
}

QString describeResponse(uint code) {
    switch (code) {
    case 0:
        return QStringLiteral("the portal reported no error");
    case 1:
        return QStringLiteral("the screenshot was cancelled");
    case 2:
        return QStringLiteral("the portal could not take the screenshot");
    default:
        return QStringLiteral("the portal returned response code %1").arg(code);
    }
}

} // namespace

/// Receives the portal's asynchronous answer.
///
/// The screenshot method replies with a request handle; the result arrives later
/// on that object's Response signal, so a receiver has to be registered before
/// waiting for it.
class PortalScreenshotListener : public QObject {
    Q_OBJECT

  public:
    bool completed = false;
    uint responseCode = 2;
    QVariantMap results;

  public slots:
    void handleResponse(uint response, const QVariantMap& payload) {
        completed = true;
        responseCode = response;
        results = payload;
        emit answered();
    }

  signals:
    void answered();
};

bool portalScreenshotRequired() {
    // Under Wayland the composited desktop cannot be read directly, and the X
    // root window belongs to XWayland and holds no screen content.
    return !qgetenv("WAYLAND_DISPLAY").isEmpty();
}

QImage takePortalScreenshot(QString* error, int timeoutMs) {
    const auto fail = [error](const QString& reason) {
        if (error != nullptr) {
            *error = reason;
        }
        return QImage();
    };

    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.isConnected()) {
        return fail(QStringLiteral("no D-Bus session bus is available"));
    }

    // A single portal round trip. The request object is named up front so the
    // answer is already being listened for before the portal can send it: the
    // portal may answer faster than the reply to the call itself arrives, and a
    // listener attached afterwards would never see it.
    const auto attempt = [&bus](int attemptTimeoutMs, QString* reason) -> QImage {
        const QString token =
            QStringLiteral("snow_shot_%1").arg(QUuid::createUuid().toString(QUuid::Id128));
        QString sender = bus.baseService();
        if (sender.startsWith(QLatin1Char(':'))) {
            sender.remove(0, 1);
        }
        sender.replace(QLatin1Char('.'), QLatin1Char('_'));
        const QString expectedPath =
            QStringLiteral("/org/freedesktop/portal/desktop/request/%1/%2").arg(sender, token);

        PortalScreenshotListener listener;
        QEventLoop loop;
        QObject::connect(&listener, &PortalScreenshotListener::answered, &loop, &QEventLoop::quit);
        if (!bus.connect(QString::fromLatin1(kPortalService), expectedPath,
                         QString::fromLatin1(kRequestInterface), QStringLiteral("Response"),
                         &listener, SLOT(handleResponse(uint, QVariantMap)))) {
            *reason = QStringLiteral("could not listen for the portal response");
            return QImage();
        }

        QDBusMessage call = QDBusMessage::createMethodCall(
            QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
            QString::fromLatin1(kScreenshotInterface), QStringLiteral("Screenshot"));
        // Asking for the desktop itself rather than the portal's own picker keeps
        // the selection UI ours, which is the whole point of the overlay.
        call << QString() << QVariantMap{{QStringLiteral("handle_token"), token},
                                         {QStringLiteral("interactive"), false}};

        const QDBusMessage reply = bus.call(call);
        if (reply.type() == QDBusMessage::ErrorMessage) {
            *reason = QStringLiteral("the portal rejected the screenshot request: %1")
                          .arg(reply.errorMessage());
            return QImage();
        }
        if (reply.arguments().isEmpty()) {
            *reason = QStringLiteral("the portal returned no request handle");
            return QImage();
        }
        const QString handlePath = reply.arguments().constFirst().value<QDBusObjectPath>().path();
        if (handlePath.isEmpty()) {
            *reason = QStringLiteral("the portal returned an empty request handle");
            return QImage();
        }
        // A portal that ignores the suggested token still answers on its own
        // object, so listen there as well.
        if (handlePath != expectedPath) {
            bus.connect(QString::fromLatin1(kPortalService), handlePath,
                        QString::fromLatin1(kRequestInterface), QStringLiteral("Response"),
                        &listener, SLOT(handleResponse(uint, QVariantMap)));
        }

        QTimer::singleShot(attemptTimeoutMs, &loop, &QEventLoop::quit);
        loop.exec();

        if (!listener.completed) {
            *reason = QStringLiteral("the portal did not answer within %1 ms").arg(attemptTimeoutMs);
            return QImage();
        }
        if (listener.responseCode != 0) {
            *reason = describeResponse(listener.responseCode);
            return QImage();
        }

        const QString uri = stringFromVariant(listener.results.value(QStringLiteral("uri")));
        if (uri.isEmpty()) {
            *reason = QStringLiteral("the portal returned no image location");
            return QImage();
        }
        const QString path = QUrl(uri).toLocalFile();
        if (path.isEmpty()) {
            *reason = QStringLiteral("the portal returned an unusable image location");
            return QImage();
        }

        QImage image;
        if (!image.load(path)) {
            *reason = QStringLiteral("could not read the screenshot the portal produced");
            return QImage();
        }
        return image;
    };

    QString reason;
    const QImage image = attempt(timeoutMs, &reason);
    if (!image.isNull()) {
        return image;
    }

    // The portal reports failures for requests it never really got going, and a
    // second try costs far less than making the user reach for the key again.
    QString retryReason;
    const QImage retried = attempt(timeoutMs / 2, &retryReason);
    if (!retried.isNull()) {
        return retried;
    }
    return fail(retryReason.isEmpty() ? reason : retryReason);
}

} // namespace snow_shot::platform

#include "portalscreenshot.moc"
