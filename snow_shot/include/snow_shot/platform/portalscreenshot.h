#ifndef SNOW_SHOT_PLATFORM_PORTALSCREENSHOT_H
#define SNOW_SHOT_PLATFORM_PORTALSCREENSHOT_H

#include <QImage>
#include <QString>

namespace snow_shot::platform {

/// Whether a screenshot has to go through the XDG Desktop Portal.
///
/// A Wayland session gives no other option: an application cannot read the
/// composited desktop directly, and the X root window belongs to XWayland and
/// holds no screen content.
[[nodiscard]] bool portalScreenshotRequired();

/// Ask org.freedesktop.portal.Screenshot for a screenshot of the whole desktop.
///
/// The portal answers asynchronously: the method returns a request handle and
/// the result arrives later on the Request object's Response signal, so this
/// waits for that signal up to `timeoutMs`. Returns a null image and writes a
/// human-readable reason to `error` when the portal is missing, refuses, or the
/// user dismisses the prompt.
[[nodiscard]] QImage takePortalScreenshot(QString* error, int timeoutMs = 30000);

} // namespace snow_shot::platform

#endif // SNOW_SHOT_PLATFORM_PORTALSCREENSHOT_H
