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
| Platform | Omarchy only (Arch + Hyprland). No portability work until it works here |
| Hardware | One USB optical drive on LG's MT1959 platform, so it can take OmniDrive later: an LG BU40N in a 9.5 mm USB-C enclosure, or an LG BP50NB40 (svc code NB50/NB52). See the [Redump OmniDrive page](https://wiki.redump.info/OmniDrive). Selling hardware is out of scope |
| Stack | Tauri 2 (Rust core, React/three.js UI). Revisit after Phase 0 if a spike argues otherwise |
| License | GPL-3.0-or-later, the same as Rainbow Player, so its code can be reused |
| Emulators | Hosted, not forked. Standalone programs or libretro cores do the emulation; Spectra identifies the disc, routes it, and supplies the UI |

## Principles

1. **Stock drive first.** Anything a normal drive can read comes before
   anything that needs OmniDrive firmware.
2. **Reuse emulators instead of writing them.** Launch existing programs
   first; host libretro cores inside the app later.
3. **Every phase ends usable.** Each phase has a clear "done when".
4. **Emulators see a file.** In the end every engine reads a disc image, and
   Spectra decides whether that image is backed by the drive, a cache or a dump.
5. **Nothing legally doubtful ships.** No BIOS files, keys or cracks. CSS and
   AACS come from the system's `libdvdcss` and `libaacs`.

## Architecture

```
┌───────────────────────── Spectra (Tauri) ─────────────────────────┐
│ UI (React/three.js): 3D disc, library, BIOS setup, gamepad remote │
├───────────────────────────────────────────────────────────────────┤
│ Rust core                                                         │
│   drive     SG_IO on /dev/sg*: TOC, READ CD, READ(12), events     │
│   identify  audio / DVD / BD / PS1 / PS2 / Sega / PC / ...        │
│   router    disc → engine, BIOS checks, per-game settings         │
│   vdisc     FUSE image backed by drive + block cache (Phase 2)    │
│   watcher   media-change events → bring app forward               │
└──────┬───────────────────────┬────────────────────────┬───────────┘
       │ separate process       │ separate process        │ library
   pcsx2, retroarch -L …,   xemu (GPL-2: always kept  libmpv (DVD/BD),
   dolphin-emu, dosbox,     out of process), wine     libcdio
   scummvm, openblack
```

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
| `src/lib/cd/discid.ts`, `musicbrainz.ts`, `cache.ts` | Reuse as is (TypeScript, UI side) |
| `src/lib/game/identify.ts`, `iso9660.ts` | Port to Rust `identify`, then extend it to more systems |
| `src/lib/game/disc.ts` | Design basis for `vdisc`: 64 KB blocks, LRU, read-ahead, keep-spinning |
| `src/lib/game/catalog.ts`, `scripts/ps2-titles.mjs` | Reuse for serial → title and cover lookups |
| `src/lib/discArt.ts`, `discScene.ts`, `components/player/*` | Reuse for the UI |
| `src/hooks/useCdAudio.ts`, `src/lib/cd/drive.ts` (audio path) | Reuse, fed by the Rust drive instead of WebUSB |
| `src/lib/input/remote.ts`, `hooks/useRemote.ts` | Reuse for gamepad and keyboard control of the menus |
| `electron/usb/virtual/*`, `src/lib/cd/virtualDrive.test.ts` | Design basis for the fake drive used in tests |
| `electron/`, `src/lib/cd/usb.ts`, `src/lib/dvd/*` decoders, `emulators/play/` | Do not port. Native access, libmpv and native emulators replace them |

## Layout

```
crates/spectra-core/    reading and identifying discs (drive over SG_IO, or images)
crates/spectra-discid/  command line: what disc is this?
app/                    the Tauri shell: React UI in app/src, Rust in app/src-tauri
docs/spikes.md          Phase 0 questions that need the drive, and their answers
scripts/check.sh        fmt, clippy, tests, frontend build
```

Rust is pinned per-project by `mise.toml`.

## Phase 0: groundwork

Answer the unknowns cheaply before building anything big.

- [ ] Buy the drive and enclosure (no OmniDrive flash yet)
- [ ] Assemble the test disc set (see [Test discs](#test-discs))
- [ ] `sudo pacman -S sg3_utils libdvdcss libaacs` and load the `sg` module
      at boot (`/etc/modules-load.d/sg.conf`)
- [x] Scaffold the repo: Cargo workspace plus the Tauri app, and
      `scripts/check.sh` for the checks. Hook it up to CI once the repo has a
      host
- [ ] **Spike: SG_IO.** Can Rust send INQUIRY, READ TOC and READ(12) to
      `/dev/sg*` as a normal user, relying only on systemd's `uaccess`?
- [ ] **Spike: PCSX2.** Does it boot a PS2 disc straight from `/dev/sr0`?
- [ ] **Spike: RetroArch.** Do Beetle PSX and Genesis Plus GX boot a physical
      disc (`cdrom://` or `/dev/sr0`)?
- [ ] **Spike: enclosure.** Does it attach as UAS or BOT, and does SG_IO
      behave differently between the two?
- [x] Build `spectra-discid`, a command-line tool that prints the media type
      and identity. Port `identify.ts` and `iso9660.ts`, and test it against
      disc images before the drive arrives. It also covers UDF (DVD and
      Blu-ray), cue sheets, MusicBrainz disc IDs, and PC Engine CD, Neo Geo
      CD, PS3, GameCube, Wii and Xbox detection
- [ ] Run `spectra-discid` against every disc in the test set on the real
      drive

**Done when:** `spectra-discid` identifies every disc in the test set, and
every spike has a written yes or no answer in `docs/spikes.md`.

## Phase 1: insert and play, stock drive

Get the core experience working for the most common discs.

- [ ] Tauri shell: fullscreen, a launcher in the Omarchy app menu, and the
      Rainbow Player UI running against the Rust drive
- [ ] Watcher: a media-change event brings Spectra forward with the disc
      shown
- [ ] Audio CD: Rainbow Player's audio path (MusicBrainz, the 3D disc,
      streaming PCM)
- [ ] DVD-Video and Blu-ray through libmpv (or an `mpv` process to start with)
- [ ] PS2 through PCSX2 as a separate process. Fall back to the libretro Play!
      core when there is no BIOS
- [ ] PS1 through `retroarch -L beetle_psx_hw`
- [ ] BIOS setup screen: drop files in, check their hashes, show what each
      system is missing
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
      CD-i, PC-FX, Amiga CD32
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

## Out of scope

| What | Why |
| --- | --- |
| PS3, PS4, PS5, Xbox One/Series, Wii U | The keys cannot be retrieved |
| Xbox 360 | Xenia on Linux is not ready |
| Dreamcast | Needs a second, specific old drive (GD-ROM) |
| Online-activation DRM | The servers are often gone |
| Other Linux distributions, macOS, Windows | Omarchy first |

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
| The enclosure's USB bridge interferes with raw commands | Test in Phase 0. Known cases: INIC-3619 bridges block flashing; the Verbatim 43888's bridge truncates some transfer sizes over USB 3 (OmniDrive issue #85), so keep raw read sizes in its safe bands or use a USB 2 cable |
| A flash bricks the drive | Phase 3 only, preferably on a second drive |
| Bus power is marginal for a slim BD drive | USB-C 10 Gbps port, a Y-cable, or a powered hub |
