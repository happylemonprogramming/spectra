# Spectra

Put a disc in the drive and it plays: music, films, and console and PC games,
from one app on Omarchy.

The name nods to [Rainbow Player](#rainbow-player), which this project borrows
from. A CD's underside is a diffraction grating that spreads light into a
spectrum, and Spectra spreads one optical drive across every disc format it
can read.

## Decisions

| Question | Decision |
| --- | --- |
| Repository | New repo. Code is pulled from Rainbow Player where it is relevant, and referenced otherwise |
| Platform | Omarchy first (Arch + Hyprland). macOS kept possible, not built: see [Portability](#portability) |
| Hardware | Phases 0-2 and 4: any name-brand USB DVD drive (about $20). It reads every CD and DVD format Spectra plays except GameCube, Wii and Xbox. Phase 3 and Blu-ray need a drive on LG's MT1959 platform, which OmniDrive runs on - ideally a used internal LG Blu-ray drive in a USB enclosure, checked against the [Redump OmniDrive page](https://wiki.redump.info/OmniDrive). Selling hardware is out of scope |
| Stack | Rust throughout. The UI is [iced](https://iced.rs) 0.14 on wgpu, with the disc as a wgpu shader. Chosen over Tauri by the UI weight spike (`docs/spikes.md`): a quarter of the memory of an empty WebKitGTK page, with the whole screen built |
| License | GPL-3.0-or-later, the same as Rainbow Player, so its code can be reused |
| Offline | Every disc Spectra supports plays without a network once it is installed. The installer sets up every engine while there is one, accepted for now over fetching an engine when a disc first needs it. Only engines tested on real discs are installed: today VLC and `libdvdcss` for DVDs, RetroArch with PCSX-ReARMed for PS1 and Spectra's Play! for PS2, about 45 MB. Another console's engine joins once one of its discs has played. `--no-engines` installs Spectra alone. Names and pictures found online are kept, so a disc looked up once looks the same offline |
| Firmware | The user's own. Spectra never ships, downloads or points to console firmware (BIOS). A user who has dumped it from their own console hands Spectra the file (`spectra firmware add`), which is recognised by its checksum and copied into RetroArch's system folder. Cores that need none come after the best core, so a disc still plays without it |
| Emulators | Hosted, with our own patched build when upstream lags. Standalone programs or libretro cores do the emulation; Spectra identifies the disc, routes it, and supplies the UI. A fix Spectra needs is sent upstream and also kept in `emulators/<name>/patches`, applied to a pinned upstream commit by `emulators/<name>/build.sh`, so users have it whether or not it is merged. A patch is dropped once upstream has it |

## Principles

1. **Stock drive first.** Anything a normal drive can read comes before
   anything that needs OmniDrive firmware.
2. **Reuse emulators instead of writing them.** Launch existing programs
   first; host libretro cores inside the app later.
3. **Every phase ends usable.** Each phase has a clear "done when".
4. **Emulators see a file.** In the end every engine reads a disc image, and
   Spectra decides whether that image is backed by the drive, a cache or a dump.
5. **Nothing legally doubtful ships.** No BIOS files, keys or cracks. CSS and
   AACS come from the system's `libdvdcss` and `libaacs`. Firmware is only
   ever the user's own, from their own console.
6. **Lean, in the Omarchy spirit.** Fast to open, nothing running when
   nothing is happening, small to install, and fine on an old laptop. Spectra
   should never be what makes a machine slow: when a game runs, the emulator
   gets the machine. The budgets below are checked, not hoped for.

## Budgets

Measured on a ThinkPad P14s Gen 6 (Ryzen AI 7 PRO 350, Radeon 860M, 60 Hz).
"Now" is the Phase 0 UI spike - the finished player screen - as a release
build.

| What | Budget | Now |
| --- | --- | --- |
| Binary size | < 10 MB | 8.7 MB (the PS2 title table adds 0.8 MB when the core is linked) |
| Launch to window on screen | < 500 ms | ~380-450 ms, about 200 of it Mesa creating a Vulkan instance |
| CPU while idle | 0% | 0% |
| Memory with the menu open | < 150 MB | 25 MB |
| Spectra's memory while a game runs | < 20 MB | not built yet |
| GPU while the menu sits idle | 0% | 0% above the desktop's own baseline |
| CPU while the disc spins | none set | ~14% of one core at 60 Hz: iced lays the whole view out every frame |
| Engines installed with Spectra | every tested one, for offline | about 45 MB: RetroArch 17 MB, VLC's DVD pieces ~14 MB, PCSX-ReARMed 2 MB, Play! 9.9 MB (built by `emulators/play/build.sh`) |

How the design keeps to them:

- **Nothing resident but a watcher.** A small Rust process (no window)
  waits for media-change events; the UI starts when a disc goes in.
- **The UI steps aside for games.** While an emulator runs, the UI process
  exits, so Spectra shrinks to the watcher and the disc cache.
- **Frames only while something moves.** The disc shows off both faces on
  arrival, then rests; a frame subscription exists only while anything is
  animating. `--reduced-motion` skips the flips.
- **Only the GPU backend in use.** wgpu starts Vulkan alone where a Vulkan
  driver is installed (OpenGL costs 90 ms of startup for nothing), and keeps
  OpenGL as the fallback on GPUs without one.
- **Computed once, not per frame.** The blurred backdrop is a 48-pixel image
  blurred on the CPU at load and stretched by the texture filter.
- **Dependencies built for size.** iced, wgpu and naga are compiled at
  `opt-level = "s"`, Spectra's own code at 3, and panics abort.
- **Video discs in a trimmed VLC.** DVD and Blu-ray go to `cvlc` (VLC with
  no interface) as a separate process, because disc menus are required and
  mpv does not do them. Only the pieces needed are installed - `vlc-cli` and
  the DVD, Blu-ray, ffmpeg, AC-3, PulseAudio and FreeType plugins: 4.3 MB to
  download, 14.6 MB installed, against 169 MB for the full `vlc` package.
  It runs through XWayland: VLC 3 cannot open a Wayland window on Hyprland,
  and Omarchy runs XWayland anyway (spike 6).
- **Every tested engine installs with Spectra.** A disc should play the
  first time it goes in, network or not, so the installer sets up every
  engine at once: the VLC pieces and `libdvdcss` for DVDs, RetroArch,
  PCSX-ReARMed from libretro's buildbot (Arch does not package it), and
  Spectra's Play! (built by `emulators/play/build.sh` for now; a ready-made build to
  download comes once the repository is public). None of it is copied into
  Spectra: `libdvdcss` is legally delicate to distribute, and system
  packages get security fixes. Installed engines cost disk, not memory or
  CPU: nothing runs until a disc does. Phase 3's engines (Dolphin, xemu)
  are larger, and are weighed again when they come.
- **Old laptops.** Spectra itself, music, films and CD-era consoles should
  run on roughly a 2012-era laptop. PS2, GameCube and Wii need a stronger CPU,
  and original Xbox a modern one; that is the emulators' floor, and Spectra
  says so when a disc is inserted rather than stuttering.

## Architecture

```
┌──────────────────────────── Spectra (Rust) ───────────────────────┐
│ UI (iced + wgpu): 3D disc, library, BIOS setup, gamepad remote    │
├───────────────────────────────────────────────────────────────────┤
│ Rust core                                                         │
│   drive     SG_IO on /dev/sg*: TOC, READ CD, READ(12), events     │
│   identify  audio / DVD / BD / PS1 / PS2 / Sega / PC / ...        │
│   router    disc → engine, BIOS checks, per-game settings         │
│   vdisc     FUSE image backed by drive + block cache (Phase 2)    │
│   watcher   media-change events → bring app forward               │
└──────┬───────────────────────┬────────────────────────┬───────────┘
       │ separate process       │ separate process        │ separate process
   pcsx2, retroarch -L …,   xemu (GPL-2: always kept  cvlc (DVD/BD, with
   dolphin-emu, dosbox,     out of process), wine     menus)
   scummvm, openblack
```

## Portability

Omarchy is the target; macOS should stay a port rather than a rewrite. Three
rules keep it that way:

1. **Heavy things are separate programs.** Video in VLC, games in their
   emulators, all of which exist on macOS. Spectra is the menu and launcher.
2. **Platform code lives in two places.** The drive backend (SG_IO here,
   IOKit's SCSITaskDeviceInterface on macOS) behind the `Disc` trait, and a
   small `platform` module for window focus, media-change events (udev,
   DiskArbitration) and paths.
3. **Everything else is cross-platform crates.** winit and wgpu (Vulkan here,
   Metal there), gilrs, cpal, `directories`, `rfd`.

What would stay weaker on macOS: the Phase 2 virtual disc (macFUSE needs a
kernel extension; FSKit or dump-first instead), and PC games (CrossOver
rather than Proton).

## Rainbow Player

Upstream: `nostr://alex@gleasonator.com/git.shakespeare.diy/rainbow-player`.
Cloned over HTTPS from
`https://git.shakespeare.diy/npub1q3sle0kvfsehgsuexttt3ugjd8xdklxfwwkh559wxckmzddywnws6cd26p/rainbow-player.git`
at commit `9404e5105f241347e6b06ba078309a71824bda95`. It is GPL-3.0-or-later.
Credit it in anything that is ported.

| Rainbow Player | Use in Spectra |
| --- | --- |
| `src/lib/cd/mmc.ts` | Port to Rust `drive`: TOC, READ CD, GET EVENT/STATUS, eject, the 150-sector offset rules |
| `src/lib/dvd/scsi.ts` | Port READ(12) and READ CAPACITY; the CSS commands are not needed (libdvdcss replaces them) |
| `src/lib/cd/bot.ts` | Reference only. Keep the lesson that sense data must be collected after a failure; SG_IO replaces the transport |
| `src/lib/cd/discid.ts` | Ported (`spectra-core::discid`) |
| `src/lib/cd/musicbrainz.ts`, `cache.ts` | Port to Rust: lookups and a disk cache under `~/.cache/spectra` |
| `src/lib/game/identify.ts`, `iso9660.ts` | Port to Rust `identify`, then extend it to more systems |
| `src/lib/game/disc.ts` | Design basis for `vdisc`: 64 KB blocks, LRU, read-ahead, keep-spinning |
| `src/lib/game/catalog.ts`, `scripts/ps2-titles.mjs` | Reuse for serial → title and cover lookups |
| `src/lib/discScene.ts` | Ported: the shaders to WGSL (`crates/spectra/src/disc.wgsl`), the motion to `motion.rs`. Bloom not yet |
| `src/lib/discArt.ts` | Port to Rust: finding the disc in a Cover Art Archive scan |
| `src/lib/library/backup.ts` (DVDs) | Ported: a DVD kept in the clear as `disc.iso`, VOB by VOB with each one's title key - through the system's libdvdcss, loaded only to copy, rather than CSS code of Spectra's own |
| `src/lib/dvd/art.ts`, `src/lib/dvdLabel.ts`, `src/lib/dvd/disc.ts` | Ported: a DVD's title and pictures from its label, Wikidata and fanart.tv (`filmdb.rs`), told apart by the feature's length (`spectra-core::dvd`). Uses Rainbow Player's fanart.tv key until Spectra has its own |
| `components/player/*` | Reference for the layout |
| `components/player/LibraryShelf.tsx`, `src/lib/shelfScene.ts`, `hooks/useWheelStep.ts` | Ported: the library below the stage (`shelf.rs`) - a grid of 3D discs, the focused one turning, the slide between the two, wheel steps, delete pressed twice |
| `components/player/ButtonGlyph.tsx`, `src/lib/input/pads.ts`, `public/input-prompts/` | Ported: prompts drawn as the pad in hand has its buttons (`ui.rs`), with Kenney's Input Prompts (CC0) rasterised to `crates/spectra/assets/prompts` |
| `src/hooks/useCdAudio.ts`, `src/lib/cd/drive.ts` (audio path) | Port: READ CD into `cpal` instead of Web Audio |
| `src/lib/input/remote.ts`, `hooks/useRemote.ts` | Ported in spirit: `Remote` in the UI, pads through `gilrs` |
| `electron/usb/virtual/*`, `src/lib/cd/virtualDrive.test.ts` | Design basis for the fake drive used in tests |
| `electron/`, `src/lib/cd/usb.ts`, `src/lib/dvd/*` decoders, `emulators/play/` | Do not port. Native access, VLC and native emulators replace them |

## VLC

Run as a program, not linked. Its source is also a reference, shallow-cloned
from `https://github.com/videolan/vlc.git` at commit `15b71e3` into
`~/Projects/vlc`. The files below are LGPL-2.1-or-later, so they can be
ported into Spectra with credit.

| VLC | Use in Spectra |
| --- | --- |
| `modules/access/dvdnav.c`, `bluray.c` | How menus, button highlights and titles are driven, if Spectra ever draws disc menus itself |
| `modules/access/cdda.c`, `vcd/cdrom.c` | CD-Text, for track names without a network |
| `modules/codec/cdg.c` | CD+G karaoke graphics (Phase 2) |
| `modules/codec/svcdsub.c`, `cvdsub.c` | SVCD and CVD subtitles (Phase 2) |
| `modules/services_discovery/udev.c` | Noticing a disc going in, for the watcher |

## Layout

```
crates/spectra-core/    reading and identifying discs (drive over SG_IO, or images)
crates/spectra-discid/  command line: what disc is this?
crates/spectra/         the app: iced UI, the disc shader, motion, gamepads, the library,
                        and the command line (`cli.rs`) that drives it over a socket (`control.rs`)
crates/spectra/assets/  Kenney's Input Prompts (CC0), as small PNGs built into the app
docs/spikes.md          Phase 0 questions and their answers
packaging/              the desktop entry and icon
scripts/check.sh        fmt, clippy, tests
scripts/measure.sh      a release build against the budgets
scripts/first-drive.sh  the first evening with a drive: spikes 1, 4 and 6
scripts/install.sh      the one-line install: builds from source under ~/.local, from a
                        checkout or piped from curl
scripts/install.ps1     the Windows one-liner, which for now says Spectra is Linux only
README.md               install and use
AGENTS.md               driving Spectra from the command line, for agents
```

Rust is pinned per-project by `mise.toml`.

## Phase 0: groundwork

Answer the unknowns cheaply before building anything big.

- [ ] Buy a USB DVD drive (the OmniDrive-capable one waits for Phase 3)
- [ ] Assemble the test disc set (see [Test discs](#test-discs))
- [x] `sudo pacman -S sg3_utils libdvdcss`, the trimmed VLC, and the `sg`
      module loaded at boot (`/etc/modules-load.d/sg.conf`). `libaacs` waits
      for a Blu-ray drive
- [x] Scaffold the repo: Cargo workspace plus the app, and
      `scripts/check.sh` for the checks. Hook it up to CI once the repo has a
      host
- [ ] **Spike: SG_IO.** Can Rust send INQUIRY, READ TOC and READ(12) to
      `/dev/sg*` as a normal user, relying only on systemd's `uaccess`?
- [ ] **Spike: PCSX2.** Does it boot a PS2 disc straight from `/dev/sr0`?
- [ ] **Spike: RetroArch.** Do Beetle PSX and Genesis Plus GX boot a physical
      disc (`cdrom://` or `/dev/sr0`)?
- [ ] **Spike: USB bridge.** Does the drive attach as UAS or BOT, which
      USB bridge chip does it use, and does SG_IO behave differently between
      the two modes?
- [x] **Spike: UI weight.** Webview or native, decided before Phase 1 ports
      the UI. Answer: native. One finished screen in iced - the rainbow disc,
      a blurred cover backdrop, a gamepad-driven track list with an animated
      focus glow - meets every budget; an empty Tauri page already missed
      the memory one. Details in `docs/spikes.md`
- [x] Build `spectra-discid`, a command-line tool that prints the media type
      and identity. Port `identify.ts` and `iso9660.ts`, and test it against
      disc images before the drive arrives. It also covers UDF (DVD and
      Blu-ray), cue sheets, MusicBrainz disc IDs, and PC Engine CD, Neo Geo
      CD, PS3, GameCube, Wii and Xbox detection
- [ ] **Spike: VLC for video discs.** Measured on test files: through
      XWayland, lighter than mpv (133 ms and 78 MB against 1.1 s and 141 MB),
      GPU decoding for Blu-ray's H.264. Left: can DVD menus be driven from
      outside VLC? Needs the DVD
- [ ] Run `spectra-discid` against every disc in the test set on the real
      drive

**Done when:** `spectra-discid` identifies every disc in the test set, and
every spike has a written yes or no answer in `docs/spikes.md`.

## Phase 1: insert and play, stock drive

Get the core experience working for the most common discs.

- [ ] The app: fullscreen, a launcher in the Omarchy app menu, and the spike
      screen running against the Rust drive and real MusicBrainz lookups
- [ ] Watcher: a media-change event brings Spectra forward with the disc
      shown
- [ ] Audio CD: Rainbow Player's audio path (MusicBrainz, the 3D disc,
      streaming PCM)
- [ ] DVD-Video and Blu-ray through `cvlc`, with their menus, driven by the
      gamepad (the watcher translates pad presses into VLC commands while the
      UI is closed)
- [ ] PS2 through PCSX2 as a separate process. Fall back to the libretro Play!
      core when there is no BIOS
- [ ] PS1 through `retroarch -L beetle_psx_hw`
- [ ] BIOS setup screen: drop files in, check their hashes, show what each
      system is missing. The checking is done, from the command line:
      `spectra firmware add FILE|FOLDER` and `spectra firmware`; the window
      says when a game needs firmware
- [ ] One gamepad-driven menu, plus a hotkey to leave a game (close the
      engine and focus Spectra through `hyprctl`)

**Not in this phase:** our own libretro host, the virtual disc, a unified
save system.

**Done when:** every disc in the test set that this phase covers plays after
you insert it, with no terminal.

## Phase 2: every CD-era system, and the virtual disc

- [ ] `vdisc`: a FUSE mount that exposes the drive as `.iso`, or as
      `.cue` + raw 2352-byte `.bin` for CDs (XA audio, CD-DA tracks), with a
      block cache, read-ahead and keep-spinning
- [ ] Copy the whole disc into the cache in the background; a disc seen
      before boots from the cache
- [ ] Move the Phase 1 engines onto `vdisc`
- [ ] Saturn (Kronos or Yabause), Sega CD, PC Engine CD, Neo Geo CD, 3DO,
      CD-i, PC-FX, Amiga CD32. Routed, untested: Beetle Saturn with the
      user's firmware or else Yabause, Genesis Plus GX and Beetle PCE with
      it, NeoCD with or without. Not installed with Spectra until one of
      their discs has played. They play kept copies, except PC Engine CD
      from the drive, as its discs carry no serial to keep a copy by. Neo
      Geo CD discs carry none either, and NeoCD reads only images, so they
      wait on a copy kept by another name
- [ ] VCD/SVCD, CD+G karaoke (R–W subchannel), CD-Text
- [ ] Library: discs seen before, with covers, save locations and per-game
      settings

**Done when:** every CD-based system in the test set works, and a PS2 disc
that has been cached loads faster than it does from the drive.

## Phase 3: the OmniDrive tier

- [ ] Flash Redump's prebuilt OmniDrive (BU40N image for slim drives):
      `redumper flash::mt1959` for a BU40N, MakeMKV 1.17.7's `sdftool` for
      drives that need cross-flashing (BP50NB40). Use a second drive if one
      is available, so a bad flash does not stop the project
- [ ] Detect the drive's capabilities (OmniDrive version) and gray out systems
      it cannot read
- [ ] Dump on first insert through redumper, with a progress screen; play from
      the image afterwards
- [ ] GameCube and Wii through Dolphin
- [ ] Original Xbox through xemu, always as a separate process
- [ ] Optional: stream straight from the drive through `vdisc` (raw reads plus
      descrambling) instead of dumping first

**Done when:** a GameCube or Xbox disc goes from insert, through one dump, to
playing.

## Phase 4: PC discs

- [ ] Detection: `AUTORUN.INF`, the volume label and file hashes, looked up in
      a disc → game table
- [ ] Engine order: an engine rewrite (OpenMW, OpenRA, OpenRCT2, devilutionX,
      fheroes2, openblack), then ScummVM or DOSBox-Staging, then a Wine or
      Proton prefix for each game
- [ ] Install once; afterwards, inserting the disc launches the game
- [ ] Protection detection (SafeDisc, SecuROM, StarForce): explain why the
      game will not run and point to a legitimate re-release. Never apply
      cracks
- [ ] Start with about 20 well-known titles; grow the table from Lutris
      install scripts, ProtonDB and PCGamingWiki

## Phase 5: our own libretro host, and polish

- [ ] An in-process libretro host: one window, Spectra's UI over the game,
      fast switching, shared save states. It replaces the RetroArch processes
- [ ] Only load cores in-process when their licenses are compatible with
      GPL-3; everything else stays a separate process
- [ ] AUR package and an Omarchy install script

**Ideas, not yet planned:** an in-game overlay, after the ModRetro
Chromatic's (`components/osd/` in
[oss-chromatic-console-mcu](https://github.com/ModRetro/oss-chromatic-console-mcu),
GPL-3): a hotkey opens a small menu over the game for brightness, colour
correction and leaving the game. Alongside it, per-system defaults for how
old games expect to look - colour correction for games tuned to washed-out
LCDs, frame blending for sprites that flicker on purpose to look
transparent, a CRT shader for console games - on without anyone having to
know the settings exist.

### Idea: a better picture for old games

Researched, not planned. Could PS1, PS2 and GameCube games look better than
they did, the way DLSS does for new ones, and the same way for every system?

- **DLSS is out.** It needs an NVIDIA RTX card, and the game engine's motion
  vectors and depth, which an emulator does not produce; the budget laptop's
  GPU is AMD's anyway. Generating each frame anew with a diffusion model
  is far too slow on an integrated GPU, and flickers from frame to frame.
- **Without a BIOS**, which Spectra avoids, each emulator gives less:
  - PS1, PCSX-ReARMed: 2× rendering (`pcsx_rearmed_neon_enhancement_enable`)
    and dithering off (`pcsx_rearmed_dithering`), both off today. No PGXP, so
    polygons still wobble. SwanStation or Beetle PSX HW would add PGXP, 4-8×
    and widescreen, but want a BIOS; PCSX-Redux's OpenBIOS, a free
    replacement, might stand in. Untested.
  - PS2, Play!: `play_res_multi` 2× or 4×. No texture replacement.
  - GameCube, Dolphin: needs no BIOS. 2-4× rendering, widescreen hacks, and
    community HD texture packs, some upscaled by AI.
- **The same for every system: post-processing.** RetroArch's slang shaders
  work on any core, given `video_driver = "vulkan"` and the shader pack
  (`libretro-shaders-slang`): `crt-guest-advanced` for the CRT look these
  games were drawn for, `fsr` (AMD FidelityFX upscaling and sharpening) or
  `nnedi3` (a small neural upscaler) for a sharper one. Spectra already
  writes a settings file for RetroArch, so a preset per system goes there.
- **In the end, Spectra's own.** With the libretro host above, every frame
  passes through Spectra's wgpu renderer, and one WGSL stage - CRT, FSR, or
  a small neural upscaler - serves every system at once, tuned per system.

The order, if it is taken up: the PS1 and PS2 options, then Vulkan and a
shader per system, then an OpenBIOS test, then a shader stage of Spectra's
own.

## On the TV

Spectra should play on the living room TV without a cable, from a computer
anywhere in the home. Nothing in Spectra draws for the TV: both paths below
send whatever is on screen, so the menu, `cvlc` and every emulator come along
for free.

| Path | What the TV needs | Good for |
| --- | --- | --- |
| Sunshine + Moonlight | The Moonlight app (Apple TV, Google TV, Android TV) | Everything, games included: low latency, and a controller paired to the TV drives Spectra through the existing gamepad remote |
| Normal casting: Google Cast, then Miracast | Nothing, it is built in | Music and films. Games feel laggy, from the encode and buffering |

AirPlay is the gap: there is no mature open-source AirPlay mirroring sender
on Linux, so an Apple TV without Moonlight cannot be reached.

- [x] Detect `sunshine` and offer Omarchy's `omarchy-install-service-sunshine`
      when it is missing; never install it unasked
- [x] "Play on TV": create a 1920×1080 headless output
      (`hyprctl output create headless`), move Spectra and the engine it
      launches there, and stream or cast that output. A straight mirror of a
      16:10 laptop letterboxes on a 16:9 TV. Done as `SPECTRA-TV`, sized as
      the TV asks when Sunshine starts it (`spectra tv on` as an app's prep
      command), with Sunshine's `output_name` naming it
- [ ] Without Sunshine, cast the output through the screencast portal
      (`gnome-network-displays`, Google Cast first, then Miracast)
- [ ] Measure input-to-photon latency on both paths with a PS2 game, and
      record it in `docs/spikes.md`
- [x] While streaming, keep the machine awake: hold a systemd inhibitor so
      the computer does not suspend mid-film

## Experiment: a game's music

On trial, merged from the `game-music` branch: a kept game's soundtrack, as a disc
of its own on the shelf beside the game - "Tomb Raider (Soundtrack)" - that
plays like an album, from the game's copy. The game is as it was. The shelf
listens to each game once a session, in the background, and the soundtrack
joins it when found; nothing is written, and it goes with its game. If it
does not earn its place, `soundtrack.rs` in both crates and the calls to
them come out together.

Tried first and dropped: the music as the game's own track list. A started
the game from a track, and picking the game started it before the music
could be reached.

- PS1: the CD audio tracks after the data, sorted by listening
  (`spectra_core::soundtrack`): silence is dropped, and so is speech -
  mono, with pauses - and anything under 20 seconds, a sting; the rest is
  music. Tomb Raider: 18 pieces of music, 6 stings, 31 lines of speech, one
  silent track
- PS2: stereo 16-bit WAV files in a music folder of the root (`MUSIC`,
  `BGM`, `SOUND`, `STREAM` and the like), heard the same way and played at
  their own rate. Midnight Club: all ten of `MUSIC/*.WAV`, whose loud mixes
  are nearly mono - which is why speech needs its pauses too
- Not done: Sony's ADPCM (`.VAG`, `.VB`), music inside a game's archives,
  and PS2 DVDs, which are not kept yet
- Not tried: naming the pieces from a published soundtrack, through
  AcoustID; matching by length is too loose, as soundtracks re-edit
- Out: films. A DVD's music is mixed in with the dialogue

## Later: macOS and Windows

Shelved, not ruled out: Omarchy comes first, and the
[Portability](#portability) rules keep both a port rather than a rewrite.

- [x] A one-line install on Linux: `curl -fsSL …/install.sh | bash` builds
      from source under `~/.local`, installing build packages (asked first)
      and Rust (rustup) where missing. macOS and Windows get a clear "Linux
      only for now"
- [ ] Put the repository's URLs into the scripts, README and AGENTS.md once
      it has a home: `__SPECTRA_REPO_URL__` (git) and `__SPECTRA_RAW_URL__`
      (raw files)
- [ ] Prebuilt binaries instead of a build on every machine: cargo-dist in CI
      builds them and writes both scripts. Start it on Linux once the repo
      has CI
- [ ] Drive backends behind the `Disc` trait: IOKit's
      SCSITaskDeviceInterface on macOS (unmount through DiskArbitration
      first), SPTI on Windows. Repeat the USB bridge spike on each
- [ ] Media-change events: DiskArbitration, and `WM_DEVICECHANGE`
- [ ] Engines from each platform's package manager: Homebrew on macOS,
      winget on Windows. VLC there bundles `libdvdcss`
- [ ] Signing: Apple's notarization for a `.app` ($99 a year), and a code
      signing certificate so SmartScreen does not warn on Windows

## Out of scope

| What | Why |
| --- | --- |
| PS3, PS4, PS5, Xbox One/Series, Wii U | The keys cannot be retrieved |
| Xbox 360 | Xenia on Linux is not ready |
| Dreamcast | Needs a second, specific old drive (GD-ROM) |
| Online-activation DRM | The servers are often gone |
| Other Linux distributions | Omarchy first |

## Throughout

- **Tests without hardware:** record SCSI traces from real discs and replay
  them through a fake drive, so CI never needs a drive. `vdisc` and
  `identify` also run against image files.
- **Licensing:** Spectra is GPL-3.0-or-later. xemu (GPL-2.0) stays a separate
  process. DuckStation (CC BY-NC-ND) is not used.
- **Legal:** users supply their own BIOS files. Nothing ships keys, cracks or
  firmware images.

## Test discs

- [ ] Audio CD, and an enhanced CD (audio plus a data track)
- [ ] DVD-Video (CSS), Blu-ray (AACS)
- [ ] PS1 (with XA audio or CD-DA music if possible)
- [ ] PS2 CD, PS2 DVD, PS2 dual-layer DVD
- [ ] Saturn or Sega CD
- [ ] A VCD or CD+G, if one is available
- [ ] A PC game with no protection, and one with SafeDisc
- [ ] Phase 3: GameCube or Wii, and an original Xbox disc

## Risks

| Risk | Mitigation |
| --- | --- |
| PCSX2 or RetroArch cannot boot a physical disc | `vdisc` (Phase 2) turns every drive into a file, so bring it forward |
| The drive's USB bridge interferes with raw commands | Test in Phase 0. Known cases: INIC-3619 bridges block flashing; the Verbatim 43888's bridge truncates some transfer sizes over USB 3 (OmniDrive issue #85), so keep raw read sizes in its safe bands or use a USB 2 cable |
| A flash bricks the drive | Phase 3 only, preferably on a second drive |
| Bus power is marginal for a slim BD drive | USB-C 10 Gbps port, a Y-cable, or a powered hub |
