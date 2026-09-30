# Phase 0 spikes

Four questions to answer before building anything big. Each needs the drive.
Record the answer in the table at the bottom, with the output that proves it.

## Setup, once

```bash
sudo pacman -S sg3_utils libdvdcss libaacs
echo sg | sudo tee /etc/modules-load.d/sg.conf   # /dev/sg* at every boot
sudo modprobe sg                                 # and now
```

Plug the drive in, then check the kernel sees it:

```bash
cargo run -q -p spectra-discid -- --list
```

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

## 4. The enclosure: UAS or BOT

```bash
lsusb -t                                       # Driver=uas or Driver=usb-storage
lsusb                                          # note the enclosure's VID:PID
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

## Results

| Spike | Answer | Evidence |
| --- | --- | --- |
| 1. SG_IO as a normal user | pending | |
| 2. PCSX2 off the drive | pending | |
| 3. RetroArch off the drive | pending | |
| 4. Enclosure mode | pending | |

Drive: model, firmware and enclosure, as `spectra-discid --list` and `lsusb`
report them:

```
pending
```
