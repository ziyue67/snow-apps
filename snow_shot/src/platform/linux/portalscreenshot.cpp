#include "snow_shot/platform/portalscreenshot.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusObjectPath>
#include <QDBusVariant>
#include <QEventLoop>
#include <QObject>
#include <QTimer>
#include <QUrl>
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

    QDBusMessage call = QDBusMessage::createMethodCall(
        QString::fromLatin1(kPortalService), QString::fromLatin1(kPortalPath),
        QString::fromLatin1(kScreenshotInterface), QStringLiteral("Screenshot"));
    call << QString() << QVariantMap();

    const QDBusMessage reply = bus.call(call);
    if (reply.type() == QDBusMessage::ErrorMessage) {
        return fail(QStringLiteral("the portal rejected the screenshot request: %1")
                        .arg(reply.errorMessage()));
    }
    if (reply.arguments().isEmpty()) {
        return fail(QStringLiteral("the portal returned no request handle"));
    }
    const QString handlePath = reply.arguments().constFirst().value<QDBusObjectPath>().path();
    if (handlePath.isEmpty()) {
        return fail(QStringLiteral("the portal returned an empty request handle"));
    }

    PortalScreenshotListener listener;
    QEventLoop loop;
    QObject::connect(&listener, &PortalScreenshotListener::answered, &loop, &QEventLoop::quit);
    const bool subscribed =
        bus.connect(QString::fromLatin1(kPortalService), handlePath,
                    QString::fromLatin1(kRequestInterface), QStringLiteral("Response"), &listener,
                    SLOT(handleResponse(uint, QVariantMap)));
    if (!subscribed) {
        return fail(QStringLiteral("could not listen for the portal response"));
    }

    QTimer::singleShot(timeoutMs, &loop, &QEventLoop::quit);
    loop.exec();

    if (!listener.completed) {
        return fail(QStringLiteral("the portal did not answer within %1 ms").arg(timeoutMs));
    }
    if (listener.responseCode != 0) {
        return fail(describeResponse(listener.responseCode));
    }

    const QString uri = stringFromVariant(listener.results.value(QStringLiteral("uri")));
    if (uri.isEmpty()) {
        return fail(QStringLiteral("the portal returned no image location"));
    }
    const QString path = QUrl(uri).toLocalFile();
    if (path.isEmpty()) {
        return fail(QStringLiteral("the portal returned an unusable image location"));
    }

    QImage image;
    if (!image.load(path)) {
        return fail(QStringLiteral("could not read the screenshot the portal produced"));
    }
    return image;
}

} // namespace snow_shot::platform

#include "portalscreenshot.moc"
