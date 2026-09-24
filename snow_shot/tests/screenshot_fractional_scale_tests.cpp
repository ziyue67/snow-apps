// Covers the desktop scale a compositor reports but does not use. A 125% GNOME Wayland
// desktop arrives as 1536x864 points at ratio 2 while the compositor hands over 1920x1080
// pixels, so geometry derived from the report claims 3072x1728 and the capture lands at the
// wrong place. The reference implementation derives every capture mapping from the frame
// that really arrived; these checks keep that derivation honest.
#include "../src/presentation/capture/captureframegeometry.h"
#include "snow_shot/presentation/screenshotcapturedisplaymodelreconciler.h"
#include "snow_shot/presentation/screenshotdisplaysession.h"
#include "snow_shot/presentation/screenshotgeometry.h"
#include "snow_shot/presentation/screenshotsourceimagecomposer.h"

#include <QApplication>
#include <QScreen>

#include <cstdlib>
#include <iostream>

namespace {
using snow_shot::presentation::capture::CapturedFrameGeometry;
using snow_shot::presentation::capture::wholeScreenFrameGeometry;

void require(bool value, const char* message) {
    if (!value) {
        std::cerr << message << '\n';
        std::exit(1);
    }
}

void fractionallyScaledFrameDerivesItsOwnScale() {
    const QRect desktop(0, 0, 1536, 864);
    const auto derived = wholeScreenFrameGeometry(QSize(1920, 1080), desktop, 2.0);
    require(derived.has_value(), "a 125% frame must derive the ratio it really has");
    require(derived->logicalRect == desktop, "the frame covers the desktop rect");
    require(std::abs(derived->backingScale - 1.25) < 0.0001,
            "the frame's own ratio is the pixel-to-point scale");
    require(derived->isValid(), "derived geometry must be usable");
}

void truthfulDisplaysKeepTheReportedGeometry() {
    const QRect desktop(0, 0, 1536, 864);
    require(!wholeScreenFrameGeometry(QSize(3072, 1728), desktop, 2.0).has_value(),
            "a truthful 2x frame needs no derivation");
    require(!wholeScreenFrameGeometry(QSize(1920, 1080), QRect(0, 0, 1920, 1080), 1.0).has_value(),
            "a 100% frame needs no derivation");
    require(!wholeScreenFrameGeometry(QSize(3071, 1727), desktop, 2.0).has_value(),
            "rounding noise must not be treated as a fractional scale");
}

void framesThatDoNotCoverTheScreenAreRejected() {
    const QRect desktop(0, 0, 1536, 864);
    require(!wholeScreenFrameGeometry(QSize(800, 600), desktop, 2.0).has_value(),
            "a window capture has its own shape and keeps the reported geometry");
    require(!wholeScreenFrameGeometry(QSize(1280, 720), desktop, 2.0).has_value(),
            "a frame smaller than the desktop cannot be a whole-screen frame");
    require(!wholeScreenFrameGeometry(QSize(), desktop, 2.0).has_value(),
            "an empty frame has no geometry");
    require(!wholeScreenFrameGeometry(QSize(1920, 1080), QRect(), 2.0).has_value(),
            "an empty desktop has no geometry");
}

void roundedRatioFrameGeometryDescribesTheDesktopBeforeTheFrameArrives() {
    const auto geometry =
        snow_shot::presentation::capture::roundedRatioFrameGeometry(QRect(0, 0, 1536, 864), 2.0);
    require(geometry.isValid() && geometry.logicalRect == QRect(0, 0, 1536, 864) &&
                geometry.backingScale == 2.0,
            "the pre-capture canvas is the desktop rect at the reported ratio");
    const auto fallback =
        snow_shot::presentation::capture::roundedRatioFrameGeometry(QRect(0, 0, 1536, 864), 0.0);
    require(fallback.backingScale == 1.0, "a missing report falls back to one pixel per point");
}

CapturedDisplayModel fractionalSnapshot(const QSize& frameSize, const QString& screenName) {
    CapturedDisplayModel snapshot;
    snapshot.stableId = QStringLiteral("display:test");
    snapshot.name = screenName;
    snapshot.physicalRect = QRect(QPoint(0, 0), frameSize);
    snapshot.image = QImage(frameSize, QImage::Format_RGBA8888);
    snapshot.image.fill(QColor(20, 40, 60));
    snapshot.image.setPixelColor(frameSize.width() - 1, frameSize.height() - 1,
                                 QColor(200, 30, 10));
    snapshot.image.setPixelColor(0, 0, QColor(10, 30, 200));
    snapshot.active = true;
    return snapshot;
}

// The capture path with the real screens of the running platform: a frame whose pixels
// disagree with the reported ratio has to become a point-based canvas whose backing scale is
// derived from the frame.
void reconcilerDerivesPointGeometryFromTheFrame() {
    QScreen* screen = QGuiApplication::primaryScreen();
    require(screen != nullptr, "the test platform must expose a screen");
    const QRect desktop = screen->geometry();
    const qreal reportedRatio = screen->devicePixelRatio() > 0 ? screen->devicePixelRatio() : 1.0;
    const QSize frameSize(qRound(desktop.width() * reportedRatio * 1.25),
                          qRound(desktop.height() * reportedRatio * 1.25));
    require(frameSize != desktop.size() * reportedRatio,
            "the fixture must disagree with the reported ratio");

    ScreenshotDisplaySession displays;
    ScreenshotCaptureDisplayModelReconciler::applySnapshots(
        displays, {fractionalSnapshot(frameSize, screen->name())});
    ScreenshotGeometryMapper geometry;
    geometry.rebuild(displays);

    const CapturedDisplayModel& display = displays.displayAt(0);
    require(display.canvasUsesPoints, "a frame that disagrees with the report uses a point canvas");
    require(display.capturedLogicalRect == desktop, "the frame covers the screen's desktop rect");
    require(std::abs(display.backingScale - reportedRatio * 1.25) < 0.001,
            "the backing scale comes from the frame, not from the report");
    require(geometry.canvasBounds() == QRectF(desktop),
            "the overlay canvas is the desktop the frame covers");

    const auto viewport = ScreenshotGeometryMapper::displayViewportGeometry(display);
    require(viewport.valid && viewport.logicalRect == desktop &&
                viewport.canvasRect == QRectF(desktop),
            "the overlay window and the canvas are the same desktop rect");
    require(std::abs(viewport.canvasToLogicalScale - 1.0) < 0.0001,
            "desktop points map one to one onto the canvas");

    const QRect selection = desktop.translated(-desktop.topLeft());
    const auto spec = screenshotSelectionRenderSpec(displays, selection);
    require(spec.isValid() && spec.pixelSize == frameSize &&
                std::abs(spec.scale - reportedRatio * 1.25) < 0.001,
            "a whole-screen selection exports the whole frame at its own resolution");

    const QImage composed = composeScreenshotSourceSelection(displays, selection);
    require(composed.size() == frameSize, "the composed capture keeps the frame's pixels");
    require(composed.pixelColor(frameSize.width() - 1, frameSize.height() - 1) ==
                QColor(200, 30, 10),
            "the far corner of the frame must land at the far corner of the selection");
    require(composed.pixelColor(0, 0) == QColor(10, 30, 200),
            "the frame must start at the selection's origin");
}

void truthfulFramesKeepThePixelCanvas() {
    QScreen* screen = QGuiApplication::primaryScreen();
    require(screen != nullptr, "the test platform must expose a screen");
    const QRect desktop = screen->geometry();
    const qreal reportedRatio = screen->devicePixelRatio() > 0 ? screen->devicePixelRatio() : 1.0;
    const QSize frameSize(qRound(desktop.width() * reportedRatio),
                          qRound(desktop.height() * reportedRatio));

    ScreenshotDisplaySession displays;
    ScreenshotCaptureDisplayModelReconciler::applySnapshots(
        displays, {fractionalSnapshot(frameSize, screen->name())});
    ScreenshotGeometryMapper geometry;
    geometry.rebuild(displays);

    const CapturedDisplayModel& display = displays.displayAt(0);
    require(!display.canvasUsesPoints, "a truthful frame keeps the pixel canvas it always had");
    require(display.canvasRect.size() == frameSize, "the canvas is the frame's pixel extent");
    const auto spec = screenshotSelectionRenderSpec(displays, display.canvasRect);
    require(spec.isValid() && spec.scale == 1.0 && spec.pixelSize == frameSize,
            "a truthful frame exports its pixels one to one");
}
} // namespace

int main(int argc, char** argv) {
    QApplication app(argc, argv);
    fractionallyScaledFrameDerivesItsOwnScale();
    truthfulDisplaysKeepTheReportedGeometry();
    framesThatDoNotCoverTheScreenAreRejected();
    roundedRatioFrameGeometryDescribesTheDesktopBeforeTheFrameArrives();
    reconcilerDerivesPointGeometryFromTheFrame();
    truthfulFramesKeepThePixelCanvas();
    std::cout << "screenshot fractional scale tests passed\n";
    return 0;
}
