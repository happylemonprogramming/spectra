#!/usr/bin/env bash
# Build Play!'s libretro core as Spectra plays it: upstream at the commit in
# ./upstream, with the fixes in ./patches that upstream has not merged yet.
#
#   emulators/play/build.sh [DIR]
#
# Puts play_libretro.so in DIR, by default Spectra's own cores folder, where
# Spectra looks before RetroArch's. The source is kept in Spectra's cache, so a
# second build only fetches and compiles what changed.
#
# Needs git, cmake, ninja and a C++ compiler (Arch: sudo pacman -S --needed git
# cmake ninja base-devel). PLAY_URL in the environment overrides the upstream,
# for a local clone.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
url_override=${PLAY_URL:-}
# shellcheck source=emulators/play/upstream
. "$here/upstream"
url=${url_override:-$PLAY_URL}

data=${XDG_DATA_HOME:-$HOME/.local/share}
cache=${XDG_CACHE_HOME:-$HOME/.cache}
out=${1:-$data/spectra/cores}
src=$cache/spectra/build/play

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() {
	printf 'build.sh: %s\n' "$*" >&2
	exit 1
}

for tool in git cmake ninja c++; do
	command -v "$tool" >/dev/null || die "needs $tool (Arch: sudo pacman -S --needed git cmake ninja base-devel)"
done

say "Fetching Play! ${PLAY_COMMIT:0:8}"
if [ ! -d "$src/.git" ]; then
	git init -q "$src"
	git -C "$src" remote add origin "$url"
fi
git -C "$src" remote set-url origin "$url"
git -C "$src" fetch -q --depth 1 origin "$PLAY_COMMIT"
# Start clean each time, so the patches apply to upstream as it is. The build
# folder stays, for a quicker second build.
git -C "$src" checkout -q -f --detach FETCH_HEAD
git -C "$src" clean -q -fdx -e /build
git -C "$src" submodule -q update --init --recursive --force --depth 1 ||
	git -C "$src" submodule -q update --init --recursive --force

for patch in "$here"/patches/*.patch; do
	git -C "$src" apply "$patch" || die "$(basename "$patch") no longer applies: see ./upstream"
done

say "Building (a few minutes the first time)"
cmake -S "$src" -B "$src/build" -G Ninja -Wno-dev \
	-DCMAKE_BUILD_TYPE=Release -DBUILD_LIBRETRO_CORE=ON -DBUILD_PLAY=OFF -DBUILD_TESTS=OFF >/dev/null
# Quiet unless it fails: the compiler warns a great deal about upstream's code.
cmake --build "$src/build" --target play_libretro >"$src/build.log" 2>&1 || {
	tail -n 40 "$src/build.log" >&2
	die "the build failed; all of it is in $src/build.log"
}

install -Dm755 "$src/build/Source/ui_libretro/play_libretro.so" "$out/play_libretro.so"
# What this core is, for when it misbehaves: the commit and each patch.
{
	echo "Play! $PLAY_COMMIT"
	for patch in "$here"/patches/*.patch; do echo "+ $(basename "$patch")"; done
} >"$out/play_libretro.txt"
# Play!'s license goes with the core.
install -Dm644 "$src/License.txt" "$out/play_libretro.License.txt"
say "Installed $out/play_libretro.so"
