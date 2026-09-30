#!/usr/bin/env bash
# Install a release build for this user, so Spectra shows up in the app
# launcher. No sudo; until there is a package, everything goes under ~/.local.
#
#   scripts/install.sh              build and install
#   scripts/install.sh --uninstall  remove what it installed
set -euo pipefail
cd "$(dirname "$0")/.."

bin=$HOME/.local/bin/spectra
desktop=$HOME/.local/share/applications/spectra.desktop
icon=$HOME/.local/share/icons/hicolor/scalable/apps/spectra.svg

if [ "${1:-}" = --uninstall ]; then
	rm -fv "$bin" "$desktop" "$icon"
	exit 0
fi

mise exec -- cargo build --release -p spectra
install -Dm755 target/release/spectra "$bin"
install -Dm644 packaging/spectra.svg "$icon"
# The full path, because a launcher's PATH need not include ~/.local/bin.
sed "s#^Exec=spectra#Exec=$bin#" packaging/spectra.desktop | install -Dm644 /dev/stdin "$desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$(dirname "$desktop")"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" || true
echo "installed $bin - search for Spectra in the app launcher"
