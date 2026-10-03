# Game title tables

Both tables are copied from [Rainbow Player](https://git.shakespeare.diy/npub1q3sle0kvfsehgsuexttt3ugjd8xdklxfwwkh559wxckmzddywnws6cd26p/rainbow-player), which
generates them; regenerate them there and copy them here.

- `ps2.json`: PS2 serials to titles and regions, from
  [PCSX2](https://github.com/PCSX2/pcsx2)'s GPL-3.0 game index
  (`scripts/ps2-titles.mjs` in Rainbow Player).
- `ps1.json`: PS1 serials to title, region, publisher and year, from
  [libretro-database](https://github.com/libretro/libretro-database)'s Redump
  and developer data (`scripts/ps1-titles.mjs` in Rainbow Player). The data,
  and so this table, is licensed
  [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Discs of a
  multi-disc set are also listed under `@<sectors>`, since every disc of a set
  carries the set's serial. The last field is the file name of a scan of the
  disc's printed side on LaunchBox's image host, from
  [LaunchBox's games database](https://gamesdb.launchbox-app.com), whose
  community scanned them; the scans themselves are fetched at run time, not
  shipped.

Covers are fetched at run time from
[xlenore/psx-covers](https://github.com/xlenore/psx-covers) and
[xlenore/ps2-covers](https://github.com/xlenore/ps2-covers).
