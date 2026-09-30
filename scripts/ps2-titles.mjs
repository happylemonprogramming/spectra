#!/usr/bin/env node
/**
 * Regenerate crates/spectra-core/data/ps2.json: PS2 disc serials to titles.
 *
 * From Rainbow Player (GPL-3.0-or-later), with the output path changed.
 *
 * A PS2 disc names itself only by the serial of its boot executable
 * (SYSTEM.CNF's `BOOT2 = cdrom0:\SCUS_971.11;1`), so a title has to come from
 * a table. PCSX2's GameIndex.yaml is the most complete one there is, and is
 * GPL-3.0 like this project. Only serial, name and region are kept; the rest
 * is emulator tuning for PCSX2.
 *
 *   node scripts/ps2-titles.mjs [path-or-url-to-GameIndex.yaml]
 */
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";

const SOURCE =
  process.argv[2] ?? "https://raw.githubusercontent.com/PCSX2/pcsx2/master/bin/resources/GameIndex.yaml";
const OUT = path.join(import.meta.dirname, "..", "crates", "spectra-core", "data", "ps2.json");

const text = /^https?:/.test(SOURCE)
  ? await (await fetch(SOURCE)).text()
  : await readFile(SOURCE, "utf8");

// The file is regular enough that a line scanner is safer than pulling in a
// YAML parser for it: every entry opens with an unindented `SERIAL:` and its
// fields are indented beneath.
const titles = {};
let serial = null;
for (const line of text.split("\n")) {
  const open = /^([A-Z]{4}-\d{5}):\s*$/.exec(line);
  if (open) {
    serial = open[1];
    titles[serial] = ["", ""];
    continue;
  }
  if (!serial || !line.startsWith("  ") || line.startsWith("   ")) continue;
  const field = /^ {2}(name|region):\s*"?(.*?)"?\s*$/.exec(line);
  if (!field) continue;
  titles[serial][field[1] === "name" ? 0 : 1] = field[2].replace(/\\"/g, '"');
}

for (const [key, [name]] of Object.entries(titles)) if (!name) delete titles[key];

await mkdir(path.dirname(OUT), { recursive: true });
await writeFile(OUT, JSON.stringify(titles));
console.log(`${Object.keys(titles).length} titles written to ${path.relative(process.cwd(), OUT)}`);
