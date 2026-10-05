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

What else helps, all optional:

| For | Install |
| --- | --- |
| Full drive access | the `sg` kernel module: `sudo modprobe sg` |
| PS1 and PS2 games | RetroArch, with the `pcsx_rearmed` and `play` cores from its Online Updater. No BIOS needed |
| Films | VLC, and `libdvdcss` for most DVDs |

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
| C | Y | Keep a copy of the disc |
| M | Select | Options for a disc; on the stage, your discs |
| Delete | | Remove a kept copy (press twice) |
| D | | The disc in the drive |
| N | | Night or day |
| , | Home | The start menu: places, settings, players, keys, quit |
| Backspace or Esc | B | Clear the search or filter, stop, or back |

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
