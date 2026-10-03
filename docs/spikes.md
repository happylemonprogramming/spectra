# Phase 0 spikes

Six questions to answer before building anything big. The first four need
the drive; the fifth does not, and the sixth only needs it at the end.
Record the answer in the table at the bottom, with the output that proves it.

## Setup, once

```bash
sudo pacman -S sg3_utils libdvdcss               # libaacs too, for Blu-ray
echo sg | sudo tee /etc/modules-load.d/sg.conf   # /dev/sg* at every boot
sudo modprobe sg                                 # and now
```

Plug the drive in and put a disc in, then run spikes 1, 4 and 6 in one go:

```bash
scripts/first-drive.sh
```

It saves everything it prints to `docs/drive-logs/`, which is the evidence for
the Results table. The sections below say what each check means, and how to
run it by hand.

## 1. SG_IO as a normal user

Can Spectra send raw commands without a custom udev rule, relying only on
systemd's `uaccess` tag?

```bash
ls -l /dev/sr0 /dev/sg*
getfacl /dev/sr0 /dev/sgN          # look for user:$USER:rw-
cargo run -q -p spectra-discid -- /dev/sgN
cargo run -q -p spectra-discid -- /dev/sr0
sg_inq /dev/sgN                    # cross-check vendor/model/firmware
```

- **Yes** if both identify a disc with no sudo.
- If only `/dev/sr0` works, the sg node lacks the `uaccess` ACL. Spectra can
  use `/dev/sr0` (it opens non-blocking and read-only if it must), but
  redumper needs `/dev/sg*`, so a one-line udev rule goes in Phase 1.

## 2. PCSX2 straight off the drive

```bash
yay -S pcsx2-latest-bin
```

PCSX2 needs a BIOS dumped from a PS2 you own (first-run wizard). Then
**System → Start Disc** and pick the drive. Also try the command line, which
Phase 1 would use: `pcsx2-qt /dev/sr0` (check `pcsx2-qt -help` for the exact
flag for a disc device, if that does not work).

- **Yes** if a PS2 disc reaches gameplay with no image on disk.
- Without a BIOS, test the fallback instead:
  `sudo pacman -S retroarch libretro-play`, then spike 3's procedure with the
  Play! core.

## 3. RetroArch cores off the drive

```bash
sudo pacman -S retroarch libretro-beetle-psx-hw libretro-genesis-plus-gx
ls /usr/lib/libretro/
```

In RetroArch: **Main Menu → Load Disc**, with a PS1 disc in (Beetle PSX needs
a PS1 BIOS in `~/.config/retroarch/system/`). Then find the command line
Phase 1 would use - I believe Load Disc opens `cdrom://drive1.cue`, which
would make it:

```bash
retroarch -L /usr/lib/libretro/<beetle psx core>.so cdrom://drive1.cue
```

- **Yes** if it boots, including CD audio or XA music playing in game.
- If Load Disc is missing from the menu, this RetroArch build has no physical
  disc support; the fallback is Phase 2's virtual disc, brought forward.

## 4. The USB bridge: UAS or BOT

```bash
lsusb -t                                       # Driver=uas or Driver=usb-storage
lsusb                                          # note the drive's VID:PID
lsusb -v -d VID:PID | grep -E 'bInterface(Class|SubClass|Protocol)'
```

Run spike 1 in whichever mode it came up in. If it is UAS, force it to BOT
and run spike 1 again, to see whether anything differs:

```bash
echo VID:PID:u | sudo tee /sys/module/usb_storage/parameters/quirks
# unplug and replug the drive
```

(The quirk lasts until reboot.) Note the subclass too: Rainbow Player's
WebUSB build only sees subclass 02, but SG_IO does not care.

A one-piece USB drive has its bridge chip inside; the VID:PID names it (for
example `152d` is JMicron, `13fd` Initio, `174c` ASMedia). Write it down:
bridges are the usual cause when raw commands fail over USB.

## 5. UI weight: webview or native

Answered without the drive. The question was whether Spectra's UI should be a
webview (Tauri, reusing Rainbow Player's React and three.js) or native.

**Answer: native, with iced 0.14 on wgpu.**

The Tauri shell, showing a nearly empty page, already cost about 220 MB
across its processes, almost all of it WebKitGTK - over the 150 MB menu budget
before any real UI existed. So the native side was built as one finished
screen rather than a bare disc (`crates/spectra`):

- Rainbow Player's disc shaders ported to WGSL: the grating-equation rainbow,
  the lacquered label with its concentric sheen, the metallised lip, the
  polycarbonate edge, Khronos PBR Neutral tone mapping. Rendered off-screen
  with 4x multisampling and composited over the UI. No bloom yet.
- Its motion: the eased flips, sway, tilt and spin-up, except that the disc
  settles after two flips instead of flipping forever.
- The album's cover, blurred, behind everything; title in Adwaita Sans; a
  track list driven by keyboard or gamepad with an animated focus glow in the
  cover's own colour.

Release build, measured with `scripts/measure.sh`
(poll `hyprctl clients` for the pid, `smaps_rollup` for PSS, `/proc/pid/stat`
for CPU, `gpu_busy_percent` for the GPU):

| | Tauri, empty page | iced, full screen |
| --- | --- | --- |
| Binary | 6.4 MB | 8.7 MB |
| Launch to window | ~300 ms | ~380-450 ms |
| Memory (PSS, all processes) | ~220 MB | 25 MB |
| CPU at rest | 0% | 0% |
| GPU at rest | - | nothing above the desktop's baseline |
| CPU while spinning | - | ~14% of one core |

Two things were tuned to get there: wgpu now starts Vulkan alone when a
Vulkan driver is installed (it was also starting OpenGL, which cost 90 ms and
the memory of a second driver), and dependencies are built at
`opt-level = "s"` with `panic = "abort"`, which took the binary from 11.9 MB
to 8.7 MB. About 200 ms of the launch is Mesa creating a Vulkan instance,
which Spectra cannot shorten; the watcher-launches-UI design hides it behind
the drive spinning up anyway.

It also answered "can it look good": the one effect iced lacks, a blur
behind a panel, was not needed - blurring the cover once on the CPU gives the
same backdrop for nothing per frame.

Left for Phase 1: bloom on the rainbow, per-frame CPU while spinning (the
whole view is laid out every frame; caching the static parts should cut it),
and a bundled font instead of relying on Adwaita Sans being installed.

## 6. VLC for video discs

DVD menus are required, and mpv does not do them (its manual: "DVD menus are
not supported"), so DVD and Blu-ray go to VLC without its interface
(`cvlc`), run as a separate process. Install only what a disc needs:

```bash
sudo pacman -S --asdeps vlc-cli vlc-plugin-dvd vlc-plugin-bluray \
  vlc-plugin-ffmpeg vlc-plugin-a52dec vlc-plugin-pulse vlc-plugin-freetype
```

That is 4.3 MB to download and 14.6 MB installed; the full `vlc` package is
169 MB.

### Measured without a disc

On test files made by ffmpeg: a DVD-format one (`-target ntsc-dvd`: MPEG-2,
AC-3, DVD navigation packets) and a Blu-ray-like one (1080p H.264 at 25
Mbit/s). Windowed, muted, 5 s of playback each.

| | VLC 3.0.23 (`cvlc`) | mpv 0.41 |
| --- | --- | --- |
| Launch to window, DVD file | 133 ms | 1110 ms |
| Memory (PSS), DVD file | 78 MB | 141 MB |
| CPU, DVD file | 8% of one core | 11% |
| Memory, 1080p H.264 | 102 MB | - |
| CPU, 1080p H.264, GPU decoding | 4% | - |
| CPU, 1080p H.264, forced software | 64% | - |
| Window | XWayland | native Wayland |

- **VLC 3 cannot open a Wayland window on Hyprland.** Its Wayland window
  module only knows the two pre-standard shell protocols (`xdg_shell` v5,
  `zxdg_shell_v6`), and Hyprland offers only the finished `xdg_wm_base`. With
  `DISPLAY` unset to force Wayland, it never shows a window and spins at 57%
  of a core. So Spectra launches it with `DISPLAY` set, through XWayland.
- **XWayland costs nothing extra here.** Omarchy runs it from login (43 MB,
  already paid), and `xwayland:force_zero_scaling` is on, so video gets the
  panel's real pixels despite the 1.25x scale.
- **GPU decoding works through XWayland** (VA-API, then `gl` output). This
  Radeon 860M has no MPEG-2 decoder, so DVDs decode on the CPU - cheaply, at
  8% of a core. Blu-ray's H.264 is on the GPU.
- **Lighter than mpv**, in both startup and memory. mpv's Vulkan renderer is
  its cost.
- **Remote control works** over a socket:
  `cvlc -I oldrc --rc-fake-tty --rc-unix SOCKET` (without `--rc-fake-tty` the
  module refuses to start unless it has a terminal). It answers `title`,
  `chapter`, `get_title` and `quit`, and has `key` for simulated key presses;
  the menu actions are `nav-up`, `nav-down`, `nav-left`, `nav-right`,
  `nav-activate` (`src/misc/actions.c`). Whether `key nav-down` moves a DVD
  menu's highlight needs a DVD.
- VLC 4 (its `master`) has a current Wayland backend, but is not released or
  packaged; revisit when Arch ships it.

### Still needs the DVD

`scripts/first-drive.sh` plays the disc with `dvd:///dev/sr0`, sends the menu
keys, and asks whether the highlight moved.

- **Yes** if the menu can be driven from outside.
- If `key nav-*` does nothing, try `key key-nav-*` (VLC 3's older action
  names), then D-Bus. The fallback is Spectra listing titles itself.

## Results

| Spike | Answer | Evidence |
| --- | --- | --- |
| 1. SG_IO as a normal user | **Yes** | `drive-logs/2026-10-03-0816.txt`: `/dev/sg0` and `/dev/sr0` both carry `user:lemon:rw-` and identify a PS1 disc without sudo; no udev rule needed |
| 2. PCSX2 off the drive | pending | |
| 3. RetroArch off the drive | **Yes, PS1 without a BIOS** | Tomb Raider (SLUS-00152) reaches its title screen with RetroArch 1.22.2 and the buildbot PCSX-ReARMed core on its HLE BIOS: `retroarch -f -L pcsx_rearmed_libretro.so cdrom://drive0.cue`. On Linux `driveN` is `/dev/sgN` (`vfs_implementation_cdrom.c`). Native Wayland window, ~23% of one core, 146 MB. CD audio and XA music not yet checked by ear |
| 4. USB bridge | **BOT, works** | Same log: LG GUD0N slim drive behind an Initio INIC-1618L bridge (`13fd:0840`), `usb-storage` at 480M, subclass 02; came up as BOT, so there was no UAS to force off |
| 5. UI weight | **Native: iced + wgpu** | Section 5 above: 25 MB vs ~220 MB, all budgets met |
| 6. VLC for video discs | **Yes so far: through XWayland** | Section 6: lighter than mpv, GPU decoding works; menus wait for a DVD |

Drive: model, firmware and USB bridge, as `spectra-discid --list` and `lsusb`
report them:

```
pending
```
