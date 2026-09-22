# Debian package configuration for Snow Shot.
#
# CPack packages an already staged install tree: the packaging entry point runs
# `cmake --install --prefix <stage>/usr` and then points CPack at that tree
# through CPACK_INSTALLED_DIRECTORIES, mirroring the NSIS flow. Every path in the
# staged tree therefore already matches its final location below /usr.
#
# Shared-library dependencies are resolved with dpkg-shlibdeps instead of being
# hard-coded, because the soname packages differ between Debian and Ubuntu and
# the optional codecs (JPEG XL, HEIF, OpenCV, FFmpeg) change package names
# between releases. dpkg-shlibdeps also keeps the dependency list honest when a
# codec is switched off in the Snow Image profile.

if(NOT TARGET snow_shot)
    message(FATAL_ERROR "SNOW_APPS_PACKAGE_SNOW_SHOT requires SNOW_APPS_BUILD_SNOW_SHOT=ON")
endif()

if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(arm64|aarch64)$")
    set(SNOW_SHOT_LINUX_ARCHITECTURE "arm64")
else()
    set(SNOW_SHOT_LINUX_ARCHITECTURE "amd64")
endif()

set(CPACK_GENERATOR "DEB")
set(CPACK_PACKAGE_FILE_NAME
    "snow-shot-${SNOW_SHOT_VERSION}-linux-${SNOW_SHOT_LINUX_ARCHITECTURE}")
set(CPACK_DEBIAN_FILE_NAME DEB-DEFAULT)
set(CPACK_DEBIAN_PACKAGE_NAME "snow-shot")
# The Debian version keeps the upstream pre-release suffix: dpkg orders
# "1.0.9-beta" above "1.0.9", which matches the release channel ordering.
set(CPACK_DEBIAN_PACKAGE_VERSION "${SNOW_SHOT_VERSION}")
set(CPACK_DEBIAN_PACKAGE_ARCHITECTURE "${SNOW_SHOT_LINUX_ARCHITECTURE}")
set(CPACK_DEBIAN_PACKAGE_MAINTAINER
    "${SNOW_SHOT_VENDOR} <${SNOW_SHOT_SUPPORT_EMAIL}>")
set(CPACK_DEBIAN_PACKAGE_HOMEPAGE "${SNOW_SHOT_HOMEPAGE_URL}")
set(CPACK_DEBIAN_PACKAGE_SECTION "graphics")
set(CPACK_DEBIAN_PACKAGE_PRIORITY "optional")
set(CPACK_DEBIAN_PACKAGE_SHLIBDEPS ON)
set(CPACK_DEBIAN_PACKAGE_CONTROL_STRICT_PERMISSION TRUE)
set(CPACK_DEBIAN_COMPRESSION_TYPE "xz")
# The staged tree already carries the GPL-3.0 and Apache-2.0 notice files under
# share/doc. Installation only has to refresh the Freedesktop caches, and dpkg
# has no equivalent of the NSIS running-application guard, so no preinst or
# prerm is generated and user data is never touched on removal.
set(CPACK_DEBIAN_PACKAGE_CONTROL_EXTRA
    "${CMAKE_CURRENT_LIST_DIR}/../snow_shot/packaging/linux/postinst"
    "${CMAKE_CURRENT_LIST_DIR}/../snow_shot/packaging/linux/postrm"
)
set(CPACK_DEBIAN_PACKAGE_DESCRIPTION
    "${SNOW_SHOT_DESCRIPTION}
 Snow Shot captures screenshots, annotates them, pins them on screen, records
 the screen, and recognizes text and QR codes. The Linux build targets X11 and
 Wayland sessions through the Qt platform plugins.")
