#!/usr/bin/env bash
# Verify the packaged Linux build end to end.
#
# Checks that the release preset configures and builds, that CPack produces a
# Debian archive with the expected layout, and that the packaged application
# starts from the installed layout on the real X11 path. Run it after changing
# the build system, the packaging rules or the application startup path.
#
# Usage: scripts/verify-snow-shot-linux.sh [preset]
set -euo pipefail

snow_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
snow_preset="${1:-snow-shot-linux-x64-release}"
snow_build_dir="$snow_repo_root/build/$snow_preset"
snow_launch_seconds="${SNOW_VERIFY_LAUNCH_SECONDS:-15}"

snow_fail() {
    printf 'verify: %s\n' "$1" >&2
    exit 1
}

snow_ok() {
    printf 'verify: ok - %s\n' "$1"
}

[[ "$snow_preset" =~ ^snow-shot-linux-x64-(release|fast)$ ]] ||
    snow_fail "expected a Linux release preset, got: $snow_preset"

export PATH="$HOME/.cargo/bin:$PATH"
export LC_ALL=C

# 1. Configure and build through the packaging entry point so the same flags,
#    staged tree and CPack settings the release uses are exercised here.
snow_package_log="$snow_repo_root/build/verify-package.log"
mkdir -p "$snow_repo_root/build"
"$snow_repo_root/scripts/package-snow-shot-linux.sh" "$snow_preset" >"$snow_package_log" 2>&1 ||
    snow_fail "packaging failed, see $snow_package_log"
snow_ok "release preset configured, built and packaged"

# 2. Inspect the produced archive without installing it.
snow_deb="$(ls -t "$snow_build_dir"/*.deb 2>/dev/null | head -1)"
[[ -n "$snow_deb" && -f "$snow_deb" ]] || snow_fail "no .deb was produced in $snow_build_dir"

dpkg-deb -I "$snow_deb" >/dev/null 2>&1 || snow_fail "control metadata is unreadable: $snow_deb"
snow_ok "archive metadata parses ($(basename "$snow_deb"), $(stat -c%s "$snow_deb") bytes)"

# 3. The installed layout must carry the executable, the codec backend and the
#    bundled Qt runtime the platform plugins resolve through $ORIGIN.
snow_contents="$(dpkg-deb -c "$snow_deb" 2>/dev/null)"
for snow_entry in ./usr/bin/snow_shot ./usr/lib/libsnow_shot_image_codec_backend.so; do
    grep -qF "$snow_entry" <<<"$snow_contents" || snow_fail "missing from the archive: $snow_entry"
done
snow_qt_libs="$(grep -cE '\./usr/lib/snow-shot/lib/libQt6.*\.so\.6' <<<"$snow_contents" || true)"
((snow_qt_libs > 0)) || snow_fail "the archive does not bundle the Qt runtime"
snow_plugins="$(grep -cE '\./usr/lib/snow-shot/plugins/platforms/' <<<"$snow_contents" || true)"
((snow_plugins > 0)) || snow_fail "the archive does not bundle the platform plugins"
snow_ok "layout carries the executable, codec backend, $snow_qt_libs Qt libraries and $snow_plugins platform plugins"

# 4. Load the extracted tree without the build host Qt on the runpath: every
#    dependency has to resolve from the package itself.
snow_root="$(mktemp -d)"
trap 'rm -rf "$snow_root"' EXIT
dpkg-deb -x "$snow_deb" "$snow_root"
snow_missing="$(ldd "$snow_root/usr/bin/snow_shot" 2>/dev/null | grep -c 'not found' || true)"
((snow_missing == 0)) || snow_fail "$snow_missing shared libraries do not resolve from the package"
snow_plugin_missing="$(ldd "$snow_root/usr/lib/snow-shot/plugins/platforms/libqxcb.so" 2>/dev/null |
    grep -c 'not found' || true)"
((snow_plugin_missing == 0)) || snow_fail "the xcb platform plugin does not resolve its Qt runtime"
snow_ok "the packaged dependencies resolve without the build host Qt"

# 5. Start it on the real X11 path. Offscreen would hide platform failures, so
#    the check uses the xcb plugin under a virtual display when one is available,
#    and falls back to offscreen only to report that the display path is unproven.
snow_run_log="$snow_repo_root/build/verify-run.log"
snow_platform=offscreen
if command -v xvfb-run >/dev/null 2>&1; then
    snow_platform=xcb
    snow_launcher=(xvfb-run -a)
    snow_display_note="under a virtual X display"
else
    # Keep the array non-empty: expanding an empty array under `set -u` fails on
    # bash older than 4.4.
    snow_launcher=(env)
    snow_display_note="offscreen (install xvfb to exercise the X11 path)"
fi

snow_status=0
(
    cd "$snow_root/usr/bin"
    QT_QPA_PLATFORM="$snow_platform" timeout "$snow_launch_seconds" \
        "${snow_launcher[@]}" ./snow_shot >"$snow_run_log" 2>&1
) || snow_status=$?

# timeout(1) reports 124 when it had to stop a still-running process, which is
# the success case here: the application reached its event loop instead of
# exiting on a failed platform or plugin load.
if ((snow_status != 124)); then
    sed -n '1,10p' "$snow_run_log" >&2 || true
    snow_fail "the packaged application exited with $snow_status on the $snow_platform platform"
fi

grep -qF 'Storage initialized' "$snow_run_log" ||
    snow_fail "the application started but never reported storage initialization"
snow_ok "the packaged application starts on the $snow_platform platform $snow_display_note"

# 6. Exercise the X11 capture backend on a display with known geometry, so a
#    regression in screen capture fails the verification instead of passing
#    unnoticed behind the packaging checks.
if [[ -d "$snow_repo_root/snow-crates" ]] && command -v xvfb-run >/dev/null 2>&1; then
    (
        cd "$snow_repo_root/snow-crates"
        timeout 900 xvfb-run -a -s "-screen 0 320x240x24" \
            cargo test -p snow-capture --test linux_capture
    ) >"$snow_repo_root/build/verify-capture.log" 2>&1 ||
        snow_fail "the X11 capture backend failed, see build/verify-capture.log"
    snow_ok "the X11 capture backend captures a frame on a virtual display"
else
    printf 'verify: note - skipped the capture backend check (needs snow-crates and xvfb)\n'
fi

# 7. The portal paths cannot reach a real desktop here, but their D-Bus exchange
#    can: a mock portal serves the same interfaces on a private bus. Tests are
#    only built by the debug preset, so this runs when that build is present and
#    says why it skipped otherwise.
snow_portal_bin="$snow_repo_root/build/snow-shot-linux-x64-debug/snow_shot/test-bin"
if [[ -x "$snow_portal_bin/snow-shot-portal-screenshot-tests" &&
    -x "$snow_portal_bin/snow-shot-global-shortcut-backend-tests" ]] &&
    command -v dbus-run-session >/dev/null 2>&1 &&
    python3 -c 'import dbus' >/dev/null 2>&1; then
    snow_portal_tmp="$(mktemp -d)"
    cat >"$snow_portal_tmp/run.sh" <<'PORTAL_ROUND_TRIP'
export SHOT_DIR="$1"
python3 "$2" >"$1/mock.log" 2>&1 &
sleep 2
status=0
SNOW_SHOT_TEST_PORTAL=1 timeout 60 "$3" >"$1/screenshot.log" 2>&1 || status=1
SNOW_SHOT_TEST_PORTAL=1 timeout 60 "$4" >"$1/shortcut.log" 2>&1 || status=1
exit $status
PORTAL_ROUND_TRIP
    if ! timeout 180 dbus-run-session -- bash "$snow_portal_tmp/run.sh" \
            "$snow_portal_tmp" "$snow_repo_root/snow_shot/tests/mock_portal.py" \
            "$snow_portal_bin/snow-shot-portal-screenshot-tests" \
            "$snow_portal_bin/snow-shot-global-shortcut-backend-tests" >/dev/null 2>&1; then
        sed -n '1,12p' "$snow_portal_tmp"/screenshot.log "$snow_portal_tmp"/shortcut.log >&2 || true
        rm -rf "$snow_portal_tmp"
        snow_fail "a portal round-trip failed, see the log above"
    fi
    rm -rf "$snow_portal_tmp"
    snow_ok "the portal screenshot and shortcut clients round-trip on a mock portal"
else
    printf 'verify: note - skipped the portal round-trips (build the debug preset, install dbus-run-session and python3-dbus)\n'
fi

printf 'verify: all checks passed\n'
