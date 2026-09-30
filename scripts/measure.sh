#!/usr/bin/env bash
# Measure a release build of Spectra against the budgets in PLAN.md.
#
#   scripts/measure.sh [ALBUM.json] [--play]
#
# Opens a window. Prints time to window, then memory, CPU and GPU after the
# disc has had time to settle (or while it spins, with --play). GPU load is
# system-wide, so a baseline with Spectra closed is printed first.
set -euo pipefail
cd "$(dirname "$0")/.."

bin=target/release/spectra
[ -x "$bin" ] || { echo "build first: cargo build --release -p spectra" >&2; exit 1; }
gpu_file=$(ls /sys/class/drm/card*/device/gpu_busy_percent 2>/dev/null | head -1)

gpu() {
	[ -n "$gpu_file" ] || { echo "n/a"; return; }
	local sum=0
	for _ in $(seq 1 50); do sum=$((sum + $(cat "$gpu_file"))); sleep 0.1; done
	echo "$((sum / 50))%"
}
ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }

settle=12
for arg in "$@"; do [ "$arg" = "--play" ] && settle=4; done

echo "GPU baseline (Spectra closed): $(gpu)"
start=$(date +%s%N)
"$bin" "$@" >/dev/null 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null' EXIT
for _ in $(seq 1 400); do hyprctl clients -j | grep -q "\"pid\": $pid" && break; sleep 0.01; done
echo "Launch to window: $(( ($(date +%s%N) - start) / 1000000 )) ms"

sleep "$settle"
pss=$(awk '/^Pss:/{print $2}' "/proc/$pid/smaps_rollup")
hz=$(getconf CLK_TCK)
a=$(ticks $pid)
g=$(gpu)
b=$(ticks $pid)
echo "Memory (PSS): $((pss / 1024)) MB"
echo "CPU over 5 s: $(awk "BEGIN { printf \"%.1f\", ($b - $a) * 100 / ($hz * 5) }")% of one core"
echo "GPU (system-wide): $g"
