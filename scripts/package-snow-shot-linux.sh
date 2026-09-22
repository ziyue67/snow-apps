#!/bin/bash
# Builds the Debian package for Snow Shot on Linux.
#
# Mirrors the macOS entry point: configure the release preset, build the
# application, stage `cmake --install` into a throwaway prefix, then let CPack
# package that staged tree. The staged prefix is /usr, so the tree CPack sees is
# exactly the tree the application expects at runtime.
set -euo pipefail
source "$(dirname "$0")/snow-build-environment.sh"

if [[ "${1:-}" == --help ]]; then
    echo 'Usage: package-snow-shot-linux.sh [snow-shot-linux-x64-release]'
    echo 'Builds the Debian package for the current Linux host.'
    exit 0
fi
[[ $# -le 1 ]] || snow_die 'Expected at most one preset. Use --help.'
[[ "$(uname -s)" == Linux ]] || snow_die 'This entry point requires Linux.'

snow_preset="${1:-snow-shot-linux-x64-release}"
[[ "$snow_preset" =~ ^snow-shot-linux-x64-(release|fast)$ ]] ||
    snow_die "Packaging requires a Linux release preset: $snow_preset"

# The Rust toolchain is provisioned outside the workspace, so make sure it wins
# over any stale system installation.
export PATH="$HOME/.cargo/bin:$PATH"
for tool in cmake ninja cargo pkg-config; do
    command -v "$tool" >/dev/null || snow_die "Missing $tool. See docs-linux-build.md."
done

# An explicit Qt6_DIR wins; the repository-local aqt kit is the convenience
# default so a checked-out workspace builds without extra environment setup.
snow_qt_dir="${Qt6_DIR:-${SNOW_QT_DIR:-$snow_repo_root/.tools/Qt/6.11.1/gcc_64/lib/cmake/Qt6}}"
[[ -f "$snow_qt_dir/Qt6Config.cmake" ]] ||
    snow_die 'Set Qt6_DIR to the Qt 6.11.1 lib/cmake/Qt6 directory.'

cd "$snow_repo_root"
cmake --preset "$snow_preset" \
    -D "Qt6_DIR=$snow_qt_dir" \
    ${SNOW_SHOT_QR_MODEL_BASE_URL:+-D "SNOW_SHOT_QR_MODEL_BASE_URL=$SNOW_SHOT_QR_MODEL_BASE_URL"}
cmake --build --preset "build-$snow_preset" --target snow_shot --parallel

# Stage the install rules under a scratch root whose layout matches the final
# package: /usr/bin, /usr/lib/<multiarch>, /usr/share. DESTDIR keeps the rules'
# absolute destinations intact while CPack reads the result from disk.
snow_stage="$snow_repo_root/build/$snow_preset/package-stage"
rm -rf "$snow_stage"
DESTDIR="$snow_stage" cmake --install "build/$snow_preset" --prefix /usr

cpack --preset "package-$snow_preset" \
    -D "CPACK_INSTALL_CMAKE_PROJECTS=" \
    -D "CPACK_INSTALLED_DIRECTORIES=$snow_stage;/"

printf 'Package written below %s\n' "$snow_repo_root/build/$snow_preset"
