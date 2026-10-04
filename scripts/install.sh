#!/usr/bin/env bash
# Install Spectra for this user, so it shows up in the app launcher and
# `spectra` works in a terminal. Linux only, for now.
#
# From anywhere, in one line:
#
#   curl -fsSL https://__SPECTRA_RAW_URL__/scripts/install.sh | bash
#
# or from a checkout:
#
#   scripts/install.sh              build and install
#   scripts/install.sh --yes        install missing build packages without asking
#   scripts/install.sh --uninstall  remove what it installed (kept copies stay)
#
# Piped, options go after `bash -s --`. SPECTRA_REPO and SPECTRA_REF pick the
# repository and branch or tag to build when not run from a checkout.
#
# No sudo for Spectra itself: it goes under ~/.local. sudo is only asked for
# to install missing build packages (a C compiler, pkg-config, the ALSA and
# udev headers), and Rust is installed with rustup when there is none.
set -euo pipefail

# Filled in once the repository has a home.
SPECTRA_REPO=${SPECTRA_REPO:-__SPECTRA_REPO_URL__}
SPECTRA_REF=${SPECTRA_REF:-main}
# let-chains and slice::as_chunks.
RUST_MIN=1.88

bin_dir=$HOME/.local/bin
data=${XDG_DATA_HOME:-$HOME/.local/share}
desktop=$data/applications/spectra.desktop
icon=$data/icons/hicolor/scalable/apps/spectra.svg
src=$data/spectra/src

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'spectra install: %s\n' "$*" >&2; exit 1; }

yes=no
uninstall=no
for arg in "$@"; do
	case $arg in
	--yes | -y) yes=yes ;;
	--uninstall) uninstall=yes ;;
	*) die "unknown option $arg" ;;
	esac
done

case $(uname -s) in
Linux) ;;
Darwin) die "Spectra runs on Linux for now. macOS needs its own drive backend; see PLAN.md, 'Later: macOS and Windows'" ;;
MINGW* | MSYS* | CYGWIN*) die "Spectra runs on Linux for now. Windows needs its own drive backend; see PLAN.md, 'Later: macOS and Windows'" ;;
*) die "Spectra runs on Linux for now, not $(uname -s)" ;;
esac

if [ "$uninstall" = yes ]; then
	rm -fv "$bin_dir/spectra" "$bin_dir/spectra-discid" "$desktop" "$icon"
	rm -rf "$src"
	echo "removed Spectra; kept copies are still in $data/spectra/library"
	exit 0
fi

# Ask on the terminal even when this script arrives through a pipe.
confirm() {
	[ "$yes" = yes ] && return 0
	[ -r /dev/tty ] || return 1
	local answer
	printf '%s [Y/n] ' "$1" >/dev/tty
	read -r answer </dev/tty || return 1
	case $answer in "" | y | Y | yes) return 0 ;; *) return 1 ;; esac
}

# --- Build packages ---------------------------------------------------------

missing_packages() {
	command -v cc >/dev/null || return 0
	command -v pkg-config >/dev/null || return 0
	pkg-config --exists alsa libudev || return 0
	[ -n "${checkout:-}" ] || command -v git >/dev/null || return 0
	return 1
}

install_packages() {
	local id like
	id=$(. /etc/os-release 2>/dev/null && echo "${ID:-}")
	like=$(. /etc/os-release 2>/dev/null && echo "${ID_LIKE:-}")
	local cmd
	case " $id $like " in
	*" arch "*) cmd="sudo pacman -S --needed base-devel pkgconf alsa-lib systemd-libs git" ;;
	*" debian "* | *" ubuntu "*) cmd="sudo apt-get install -y build-essential pkg-config libasound2-dev libudev-dev git" ;;
	*" fedora "* | *" rhel "*) cmd="sudo dnf install -y gcc pkgconf-pkg-config alsa-lib-devel systemd-devel git" ;;
	*" suse "* | *" opensuse "*) cmd="sudo zypper install -y gcc pkg-config alsa-devel libudev-devel git" ;;
	*) die "missing build packages: a C compiler, pkg-config, git, and the ALSA and libudev headers. Install them with your package manager and run this again" ;;
	esac
	say "Spectra needs some build packages:"
	echo "  $cmd"
	confirm "Install them now?" || die "install them, then run this again"
	$cmd
}

# --- The source ---------------------------------------------------------------

# Run from a checkout, or piped from curl.
here=${BASH_SOURCE[0]:-}
checkout=
if [ -n "$here" ] && [ -f "$here" ]; then
	root=$(cd "$(dirname "$here")/.." && pwd)
	[ -f "$root/crates/spectra/Cargo.toml" ] && checkout=$root
fi

missing_packages && install_packages

if [ -n "$checkout" ]; then
	cd "$checkout"
else
	case $SPECTRA_REPO in
	__*__) die "this script does not know where Spectra's repository is yet: set SPECTRA_REPO to its git URL" ;;
	esac
	if [ -d "$src/.git" ]; then
		say "Updating the source in $src"
		git -C "$src" fetch --depth 1 origin "$SPECTRA_REF"
		git -C "$src" checkout -q --force FETCH_HEAD
	else
		say "Fetching the source into $src"
		mkdir -p "$(dirname "$src")"
		git clone -q --depth 1 --branch "$SPECTRA_REF" "$SPECTRA_REPO" "$src"
	fi
	cd "$src"
fi

# --- Rust -----------------------------------------------------------------------

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
cargo=(cargo)
# The project pins its Rust with mise; use it where it is installed.
if command -v mise >/dev/null && [ -f mise.toml ]; then
	cargo=(mise exec -- cargo)
elif ! command -v cargo >/dev/null; then
	say "Installing Rust with rustup (into ~/.rustup and ~/.cargo)"
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
		sh -s -- -y --profile minimal --no-modify-path
	. "$HOME/.cargo/env"
fi

version=$("${cargo[@]}" --version | awk '{print $2}')
if [ "$(printf '%s\n%s\n' "$RUST_MIN" "$version" | sort -V | head -1)" != "$RUST_MIN" ]; then
	command -v rustup >/dev/null || die "Rust $version is too old: Spectra needs $RUST_MIN or newer"
	say "Rust $version is too old; updating"
	rustup update stable
fi

# --- Build and install ----------------------------------------------------------

say "Building Spectra (a few minutes the first time)"
# Here, whatever the user's own setting, so the binaries are where they are
# copied from.
export CARGO_TARGET_DIR=$PWD/target
"${cargo[@]}" build --release --locked -p spectra -p spectra-discid

install -Dm755 target/release/spectra "$bin_dir/spectra"
install -Dm755 target/release/spectra-discid "$bin_dir/spectra-discid"
install -Dm644 packaging/spectra.svg "$icon"
# The full path, because a launcher's PATH need not include ~/.local/bin.
sed "s#^Exec=spectra#Exec=$bin_dir/spectra#" packaging/spectra.desktop | install -Dm644 /dev/stdin "$desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$(dirname "$desktop")"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true

say "Installed $bin_dir/spectra"
case ":$PATH:" in
*":$bin_dir:"*) ;;
*) echo "  $bin_dir is not on your PATH: add it to use \`spectra\` in a terminal" ;;
esac

# --- What else helps ------------------------------------------------------------

notes=()
ls /dev/sg* >/dev/null 2>&1 ||
	notes+=("Load the sg module for full drive access: sudo modprobe sg, and to keep it: echo sg | sudo tee /etc/modules-load.d/sg.conf")
command -v retroarch >/dev/null ||
	notes+=("For games, install RetroArch, then the pcsx_rearmed (PS1) core from its Online Updater")
[ -f "$data/spectra/cores/play_libretro.so" ] ||
	notes+=("For PS2 games, build Play! with Spectra's fixes: $PWD/emulators/play/build.sh (needs cmake and ninja)")
command -v vlc >/dev/null ||
	notes+=("For films, install VLC, and libdvdcss for most DVDs")
if [ ${#notes[@]} -gt 0 ]; then
	echo
	for note in "${notes[@]}"; do echo "  - $note"; done
fi
echo
echo "Open it from the app launcher, or: spectra open"
echo "Put a disc in and it plays. 'spectra --help' lists the commands."
