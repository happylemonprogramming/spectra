# Spectra, for agents

Spectra is a disc player for Linux: music CDs, DVDs and PlayStation games,
from the drive or from copies kept in its library. It is a window, and the
same window can be driven from the command line. This file is how to do
that. Working on the code instead: see [Changing the code](#changing-the-code).

## Is it installed?

```sh
command -v spectra || curl -fsSL https://__SPECTRA_RAW_URL__/scripts/install.sh | bash -s -- --yes
```

`--yes` installs missing build packages with sudo without asking; leave it
out to let the user decide. The binary goes to `~/.local/bin/spectra`.

## Commands

| Command | Does |
| --- | --- |
| `spectra open` | Open the window in the background and return. Does nothing if it is open |
| `spectra library [--json]` | List kept copies. Reads files only: works without the window |
| `spectra play [disc \| ID \| TITLE] [--track N]` | Put something on the stage and play it. Opens the window first if needed |
| `spectra pause` / `resume` / `toggle` | Pause and resume what plays |
| `spectra next` / `previous` | The next or previous track (a chapter, for a film) |
| `spectra stop` | Stop the music, or close the film |
| `spectra status [--json]` | What is on the stage, what plays, the drive |
| `spectra quit` | Close the window |

`play` takes:

- **nothing**: whatever is on the stage now - resumes it if paused, or plays
  the focused track, or starts the game or film on it.
- **`disc`**: the disc in the drive. Music plays from track 1, a game starts
  in RetroArch, a film starts in VLC.
- **an ID** from `spectra library` (`cd-…` for albums, a serial such as
  `SLUS-00152` for games), or **words** from the title or artist, any case:
  `spectra play semi-charmed`. Words that match more than one copy are an
  error that lists the matches; use the ID then.

`--track N` (or `-t N`) counts from 1. Every control command takes `--json`.

### Exit status

| Code | Meaning |
| --- | --- |
| 0 | Done |
| 1 | Refused or failed: the reason is on stderr, or in `error` with `--json` |
| 2 | Bad usage, or `play` named nothing in the library |
| 3 | Spectra is not running (every command but `open`, `library` and `play`) |

`play` waits a moment after the window answers and checks the sound really
started, so exit 0 means the music is playing.

## JSON

`spectra library --json`: an array, newest first.

```json
[{ "id": "cd-Pz1GkG9FBzjSGqgRz2EMHCW1qT4-", "kind": "music",
   "title": "Third Eye Blind", "artist": "Third Eye Blind", "system": null,
   "year": "1997-04-08",
   "tracks": [{ "number": 1, "title": "Losing a Whole Year", "seconds": 200 }] },
 { "id": "SLUS-00152", "kind": "game", "title": "Tomb Raider",
   "artist": null, "system": "PlayStation", "year": "1996", "tracks": [] }]
```

Every control command with `--json`:

```json
{ "ok": true, "running": true,
  "status": {
    "screen": "stage",          // or "library"
    "source": "copy",           // "drive", "copy", or "file" (an album file)
    "drive": "empty",           // "no-drive", "empty", "reading", "disc", "unreadable", "not-watched"
    "id": "cd-Pz1GkG9F…",       // library ID of what is on the stage, if any
    "kind": "music",            // "music", "game", "film", "none"
    "title": "Third Eye Blind", "artist": "Third Eye Blind",
    "tracks": [{ "number": 1, "title": "Losing a Whole Year", "seconds": 200 }],
    "playing": 3,               // track number, or null
    "paused": false, "elapsed": 42,
    "in_game": false, "in_film": false, "copying": false,
    "note": "Your copy  ·  Backspace Stop, then Backspace again to the drive"
  } }
```

On failure `ok` is false and `error` says why; `status` is still there when
the window answered. Not running: `{"ok":false,"running":false,"error":…}`.

## Recipes

Play an album by name:

```sh
spectra library                       # find it
spectra play "third eye blind" --track 3
spectra status                        # "Playing 3/14: Semi-Charmed Life (0:04 of 4:28)  ·  Third Eye Blind - …"
```

Play whatever disc the user put in:

```sh
spectra open
spectra status --json                 # wait while "drive" is "reading"
spectra play disc
```

A disc takes up to half a minute to read after it goes in: poll `status`
every couple of seconds until `drive` is `disc` (or `empty` /
`unreadable`) before `play disc`.

## What to know

- **It is the user's screen and speakers.** Spectra opens a window and plays
  sound on the user's desktop. Games and films take the whole screen. Only
  play things when asked.
- **Games and films are someone else's window.** While `in_game` is true,
  commands are refused until the user quits the game (hold Start, or Esc
  twice). While `in_film` is true, pause/next/previous/stop drive VLC.
- **Keeping a copy** (C in the window) reads the whole disc, minutes for a
  CD; while `copying` is true commands are refused. Keeping and deleting
  copies are the user's choices: `spectra-discid --keep` keeps one from a
  terminal, but ask before running it, and never delete from the library.
- **One window.** Commands go to the running window over a Unix socket,
  `$XDG_RUNTIME_DIR/spectra.sock`, which only the user can reach.
- **A display is needed.** `open` and `play` start a window, so they need
  `WAYLAND_DISPLAY` or `DISPLAY` from the user's session. A window started
  by a command logs to `~/.cache/spectra/spectra.log`.
- **Where things are.** Kept copies: `~/.local/share/spectra/library/<id>/`
  (`disc.bin`, `disc.cue`, `meta.json`). Caches and logs: `~/.cache/spectra/`
  (`game.log` for RetroArch, `film.log` for VLC).
- **What a disc is**, without the window: `spectra-discid` (`--list` for the
  drives, `--json` for detail).

## Changing the code

Rust workspace, pinned by `mise.toml`:

```
crates/spectra-core/    reading and identifying discs: SG_IO drive, cue/bin images, the library
crates/spectra-discid/  command line: what disc is this?
crates/spectra/         the app (iced on wgpu)
  src/main.rs           the window: state, the remote, the stage and the library
  src/cli.rs            the command line above
  src/control.rs        the socket the command line talks to; Request and Status
  src/audio.rs          CD audio through cpal; game.rs RetroArch; film.rs VLC
scripts/check.sh        fmt, clippy -D warnings, tests: run before calling work done
scripts/install.sh      the installer
```

- Read [PLAN.md](PLAN.md) first: principles (lean, nothing legally doubtful,
  emulators hosted not forked), budgets, and the phases.
- A new command: a `Request` variant in `control.rs`, its handling in
  `Spectra::control` in `main.rs`, and its words in `cli.rs` and here.
- Comments are plain sentences about why, in the voice of the ones around
  them. Tests are named as sentences.
