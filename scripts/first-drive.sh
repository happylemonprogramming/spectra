#!/usr/bin/env bash
# The first evening with a drive: spikes 1, 4 and 6 of docs/spikes.md.
#
#   scripts/first-drive.sh [/dev/srN]
#
# Plug the drive in and put a disc in first; a DVD-Video disc also runs the
# VLC checks, which ask a couple of questions about what is on screen.
# Everything printed is saved under docs/drive-logs/, with serial numbers
# left out.
set -uo pipefail
cd "$(dirname "$0")/.."

mkdir -p docs/drive-logs
log="docs/drive-logs/$(date +%Y-%m-%d-%H%M).txt"
exec > >(grep --line-buffered -v -i -E "serial|iSerial" | tee "$log") 2>&1

section() { printf '\n== %s\n' "$*"; }
ask() {
	local answer
	printf '%s [y/n] ' "$1" >/dev/tty
	read -r answer </dev/tty
	echo "$1 $answer"
	[ "$answer" = y ]
}

section "Build"
mise exec -- cargo build -q --release -p spectra-discid || exit 1
discid=target/release/spectra-discid

section "Drives the kernel knows about"
$discid --list
sr=${1:-$(ls /dev/sr* 2>/dev/null | head -1)}
[ -n "$sr" ] && [ -e "$sr" ] || { echo "no /dev/sr* - is the drive plugged in?"; exit 1; }
sg=$(ls "/sys/block/$(basename "$sr")/device/scsi_generic/" 2>/dev/null | head -1)
sg=${sg:+/dev/$sg}
echo "block node: $sr   generic node: ${sg:-none (is the sg module loaded?)}"

section "Spike 4: the USB bridge"
udevadm info -q property -n "$sr" |
	grep -E "^ID_(BUS|VENDOR|VENDOR_ID|MODEL|MODEL_ID|REVISION|USB_DRIVER|USB_INTERFACES|CDROM[A-Z_]*)="
echo "(ID_USB_DRIVER: uas = UAS, usb-storage = BOT)"
lsusb -t | grep -E "Class=Mass Storage" || true

section "Spike 1: raw commands as a normal user"
for node in "$sr" $sg; do
	ls -l "$node"
	getfacl -p "$node" 2>/dev/null | grep -E "^user:" || echo "(no per-user ACL on $node)"
done
[ -n "$sg" ] && sg_inq "$sg" | grep -E "Vendor|Product|revision|Peripheral device type"
spike1=yes
for node in $sg "$sr"; do
	echo "-- spectra-discid $node"
	$discid "$node" || { echo "FAILED on $node (exit $?)"; spike1=no; }
done
echo "spike 1 (both nodes identify the disc without sudo): $spike1"

section "The disc"
report=$($discid --json "$sr")
echo "$report"
kind=$(jq -r '.kind // empty' <<<"$report")

if [ "$kind" != dvd-video ]; then
	section "Spike 6 skipped: this is a '${kind:-unknown}' disc, not DVD-Video"
	echo "log: $log"
	exit 0
fi

section "Spike 6: the DVD in VLC"
echo "VLC will open full screen with the disc's menu. Leave it alone until asked."
sock=$(mktemp -u /tmp/spectra-vlc-XXXX.sock)
rc() { printf '%s\n' "$*" | socat -t1 - "UNIX-CONNECT:$sock" 2>/dev/null; }
ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
start=$(date +%s%N)
cvlc --fullscreen --no-video-title-show -I oldrc --rc-fake-tty --rc-unix "$sock" \
	"dvd://$sr" >/tmp/spectra-vlc.log 2>&1 &
vlc=$!
trap 'kill $vlc 2>/dev/null; rm -f "$sock"' EXIT
for _ in $(seq 1 3000); do
	win=$(hyprctl clients -j | jq -c --argjson p $vlc '.[] | select(.pid == $p) | {xwayland, class}')
	[ -n "$win" ] && break
	sleep 0.01
done
echo "launch to window: $(( ($(date +%s%N) - start) / 1000000 )) ms (includes the disc spinning up)  $win"
sleep 10
a=$(ticks $vlc); sleep 5; b=$(ticks $vlc)
echo "at the menu: PSS $(( $(awk '/^Pss:/{print $2}' /proc/$vlc/smaps_rollup) / 1024 )) MB," \
	"CPU $(awk "BEGIN { printf \"%.1f\", ($b - $a) * 100 / ($(getconf CLK_TCK) * 5) }")% of one core"
echo "VLC reports: $(rc title | tr -d '\r' | grep -v returned | tail -1)"

ask "Is the disc's menu on screen?" || echo "(no menu: some discs play a trailer first - note that)"
menu=no
for prefix in "" "key-"; do
	rc key "${prefix}nav-down"
	sleep 1
	if ask "Sent '${prefix}nav-down'. Did the highlighted button move?"; then
		menu="yes, with 'key ${prefix}nav-*'"
		rc key "${prefix}nav-activate"
		sleep 2
		ask "Sent '${prefix}nav-activate'. Did the highlighted item start?" || menu="$menu, but activate did nothing"
		break
	fi
done
echo "spike 6 (menus driven from outside VLC): $menu"

a=$(ticks $vlc); sleep 5; b=$(ticks $vlc)
echo "playing: PSS $(( $(awk '/^Pss:/{print $2}' /proc/$vlc/smaps_rollup) / 1024 )) MB," \
	"CPU $(awk "BEGIN { printf \"%.1f\", ($b - $a) * 100 / ($(getconf CLK_TCK) * 5) }")% of one core"
rc quit >/dev/null
grep -E "using (hw decoder|vout display) module|dvdcss|libdvdnav: (Using|DVD Title)" /tmp/spectra-vlc.log | sort -u | head

section "Done"
echo "log: $log - check it, then record the answers in docs/spikes.md"
