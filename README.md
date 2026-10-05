# Spectra

Put a disc in the drive and it plays: music, films, and PlayStation games,
from one app. Spectra reads the disc itself, names it (MusicBrainz for CDs,
its serial for games, Wikidata for DVDs), and hands it to whatever plays it:
its own CD player, VLC, or RetroArch. Any disc can be kept as a copy, to play
later without the disc.

Linux only, for now. macOS and Windows need drive backends of their own; see
[PLAN.md](PLAN.md#later-macos-and-windows).

## Install

```sh
curl -fsSL https://__SPECTRA_RAW_URL__/scripts/install.sh | bash
```

That builds Spectra from source and installs it for your user under
`~/.local`: no sudo, except to install missing build packages (a C compiler,
pkg-config, the ALSA and udev headers), which it asks about first. Rust comes
from rustup when there is none. Arch, Debian/Ubuntu, Fedora and openSUSE are
recognised.

From a checkout, the same script:

```sh
scripts/install.sh               # build and install
scripts/install.sh --uninstall   # remove it; kept copies stay
```

The installer also sets up what plays discs, after asking, so that they
play offline from then on: about 45 MB. Only what has been tested on real
discs is installed. On Arch:

| For | What |
| --- | --- |
| Music CDs | Nothing more: Spectra plays them itself |
| DVDs | VLC without its interface, its DVD pieces, and `libdvdcss` |
| PS1 games | RetroArch, with the `pcsx_rearmed` core |
| PS2 games | RetroArch, with Play! built with Spectra's fixes |

Saturn, Sega CD, PC Engine CD and Neo Geo CD games are routed but untested:
with RetroArch's `mednafen_saturn` (or `yabause`), `genesis_plus_gx`,
`mednafen_pce` or `neocd` core installed by hand, and for most of them the
console's firmware from your own console (see [Firmware](#firmware)), they
may play.

`--no-engines` installs Spectra alone. Elsewhere, it says what to install.
For full drive access, load the `sg` kernel module: `sudo modprobe sg`.

On macOS and Windows the one-liners say Spectra is not there yet:
`irm https://__SPECTRA_RAW_URL__/scripts/install.ps1 | iex` on Windows.

## Use

Open Spectra from the app launcher and put a disc in. Keyboard or gamepad:

| Key | Pad | Does |
| --- | --- | --- |
| Enter | A | Open a disc; on the stage, play the track, game or film |
| Space | Start | Play / pause |
| Page Up / Down | LB / RB | Previous / next track; among your discs, the next filter |
| S, V, G | | Show only Sounds, Videos or Games; again for all |
| Shift+F | | Show only Favorites; again for all |
| / or Ctrl+F | | Search your discs |
| F | X | Favorite, or not |
| C | Y | Keep a copy of the disc; on a kept game, its soundtrack; on that, save it to Music |
| M | Select | Options for a disc; on the stage, your discs |
| Delete | | Remove a kept copy (press twice) |
| D | | The disc in the drive |
| N | | Night or day |
| , | Home | The start menu: places, settings, players, keys, quit |
| Backspace or Esc | B | Clear the search or filter, stop, or back |
| T | | Get the TV ready, or stop |

A kept game with music on its disc - CD audio, or plain WAV files - has its
soundtrack beside it among your discs, and a Soundtrack button on its stage.
It plays as an album does. Save to Music writes each track to your Music
folder as a FLAC file, tagged and with the game's cover, to play anywhere.

## On the TV

With [Sunshine](https://github.com/LizardByte/Sunshine) on the computer and
Moonlight on the TV, Spectra plays on the TV while the computer's own screen
stays yours. Press T: Spectra starts Sunshine if it is not running and says
when it is ready. Open Spectra in Moonlight on the TV, and only then does
Spectra move to a screen of its own, as large as the TV asks for, with the
films and games it starts. End the stream and it comes back; T again stops
Sunshine. Opening Spectra from the launcher while it is on the TV brings it
back too.

Once, Sunshine is pointed at Spectra's screen, in
`~/.config/sunshine/sunshine.conf`:

```
capture = wlr
output_name = SPECTRA-TV
```

and Spectra is added to Sunshine's apps, with `spectra tv on` to do and
`spectra tv off` to undo. Hyprland only, for now.

## Firmware

Most consoles start from firmware, a BIOS, which belongs to their maker.
Spectra never provides it. PS2 games and Neo Geo CD games play without it;
PS1 and Saturn games play without it too, but more of them play with it;
Sega CD and PC Engine CD games need it.

If you have dumped it from a console of your own, hand Spectra the files,
or a folder of them, by any name:

```sh
spectra firmware add ~/bios      # copies what it recognises into RetroArch's system folder
spectra firmware                 # what is there, and which games need what
```

Files are recognised by their checksums, the ones libretro publishes. Other
files are left alone, and nothing already there is overwritten.

## From a terminal, or an agent

The window can be driven from the command line, so scripts and AI agents can
play things too. See [AGENTS.md](AGENTS.md).

```sh
spectra library                  # what is kept
spectra play "semi-charmed" -t 3 # by title or artist, from track 3
spectra play disc                # whatever is in the drive
spectra pause; spectra next; spectra status
spectra play "blue lines" --new  # a second window, playing beside the first
spectra windows                  # every window; commands take --window PID
spectra tv on                    # over to the TV, through Sunshine; tv off, back
spectra firmware add ~/bios      # a console's firmware, from your own console
spectra quit
```

Several windows can be open at once, a film in each. Only the one last
looked at hears the gamepad, and the drive serves one window at a time.

## Develop

```sh
cargo run -p spectra                       # the app
cargo run -p spectra -- album.json --play  # with an album file instead of a drive
scripts/check.sh                           # fmt, clippy, tests
```

Rust is pinned by `mise.toml`. [PLAN.md](PLAN.md) is the design: decisions,
budgets, the architecture and what comes next.

GPL-3.0-or-later. Spectra borrows from [Rainbow Player](PLAN.md#rainbow-player).
