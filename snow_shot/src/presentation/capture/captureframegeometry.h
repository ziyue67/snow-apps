#ifndef SNOW_SHOT_CAPTUREFRAMEGEOMETRY_H
#define SNOW_SHOT_CAPTUREFRAMEGEOMETRY_H
#include "snow_capture.h"
#include <QRect>
#include <QSize>
#include <cmath>
#include <limits>
#include <optional>
namespace snow_shot::presentation::capture {
inline std::optional<QRect> logicalFrameRect(const SnowCaptureFrameGeometry& geometry) {
    const double minimum = std::numeric_limits<int>::min();
    const double maximum = std::numeric_limits<int>::max();
    if (geometry.coordinate_space != 1 || !std::isfinite(geometry.x) ||
        !std::isfinite(geometry.y) || !std::isfinite(geometry.width) ||
        !std::isfinite(geometry.height) || !std::isfinite(geometry.backing_scale) ||
        geometry.backing_scale <= 0 || geometry.width < 1 || geometry.height < 1 ||
        geometry.width > maximum || geometry.height > maximum || geometry.x < minimum ||
        geometry.y < minimum || geometry.x + geometry.width > maximum ||
        geometry.y + geometry.height > maximum)
        return std::nullopt;
    return QRect(qRound(geometry.x), qRound(geometry.y), qRound(geometry.width),
                 qRound(geometry.height));
}

/// The scale a compositor really used, derived from the pixels it handed over and the
/// desktop rect those pixels cover. A fractionally scaled Wayland desktop reports a
/// rounded device pixel ratio, so the reported ratio cannot describe the frame: 125%
/// arrives as a 1536x864 desktop at ratio 2 while the compositor delivers 1920x1080
/// pixels. The reference implementation derives every capture mapping from the frame
/// that actually arrived instead of from the report.
struct CapturedFrameGeometry {
    QRect logicalRect;
    double backingScale = 1.0;

    [[nodiscard]] bool isValid() const {
        return !logicalRect.isEmpty() && std::isfinite(backingScale) && backingScale > 0.0;
    }
};

/// Geometry of a whole-screen frame: the screen's desktop rect plus the ratio the frame
/// really has. Absent when the frame is not a whole-screen frame (a window or region
/// capture keeps the reported-ratio geometry) or when the reported ratio already
/// describes the frame, so truthful displays are left untouched.
inline std::optional<CapturedFrameGeometry>
wholeScreenFrameGeometry(const QSize& framePixelSize, const QRect& screenLogicalGeometry,
                         double reportedRatio, double tolerance = 0.01) {
    if (framePixelSize.isEmpty() || !screenLogicalGeometry.isValid() ||
        screenLogicalGeometry.isEmpty()) {
        return std::nullopt;
    }

    const double scaleX =
        static_cast<double>(framePixelSize.width()) / screenLogicalGeometry.width();
    const double scaleY =
        static_cast<double>(framePixelSize.height()) / screenLogicalGeometry.height();
    if (!std::isfinite(scaleX) || !std::isfinite(scaleY) || scaleX <= 0.0 || scaleY <= 0.0) {
        return std::nullopt;
    }
    // A whole-screen frame can never be smaller than the desktop rect it covers.
    if (framePixelSize.width() < screenLogicalGeometry.width() ||
        framePixelSize.height() < screenLogicalGeometry.height()) {
        return std::nullopt;
    }
    // Only a frame with the screen's shape is a whole-screen frame.
    if (std::abs(scaleX - scaleY) > tolerance * std::max(scaleX, scaleY)) {
        return std::nullopt;
    }
    // The reported ratio already describes this frame, so nothing has to be derived.
    if (std::isfinite(reportedRatio) && reportedRatio > 0.0 &&
        std::abs(scaleX - reportedRatio) <= tolerance * reportedRatio) {
        return std::nullopt;
    }

    CapturedFrameGeometry geometry;
    geometry.logicalRect = screenLogicalGeometry;
    geometry.backingScale = scaleX;
    return geometry;
}

/// Geometry of a frame that covers a screen reporting its rounded ratio, used before the
/// compositor hands over any pixels: the desktop rect stays the canvas and the ratio the
/// report claims is the only estimate available until the frame replaces it.
inline CapturedFrameGeometry roundedRatioFrameGeometry(const QRect& screenLogicalGeometry,
                                                       double reportedRatio) {
    CapturedFrameGeometry geometry;
    geometry.logicalRect = screenLogicalGeometry;
    geometry.backingScale =
        std::isfinite(reportedRatio) && reportedRatio > 0.0 ? reportedRatio : 1.0;
    return geometry;
}
} // namespace snow_shot::presentation::capture
#endif
