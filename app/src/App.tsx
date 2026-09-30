import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface DriveInfo {
  generic: string | null;
  block: string | null;
  vendor: string;
  model: string;
  revision: string;
}

/** Mirrors spectra_core::Report, flattened the way serde writes it. */
type Report = {
  source: string;
  media: string;
  label: string | null;
  sectors: number | null;
} & (
  | { kind: "audio"; tracks: number; enhanced: boolean; musicbrainz: { disc_id: string; toc: string } | null }
  | { kind: "game"; system: string; serial: string | null; title: string | null; region: string | null }
  | { kind: "dvd-video" | "blu-ray-video" | "pc" | "data" }
  | { kind: "video-cd"; super_vcd: boolean }
);

function describe(r: Report): string {
  switch (r.kind) {
    case "audio":
      return `Audio CD, ${r.tracks} tracks${r.enhanced ? " (enhanced)" : ""}`;
    case "game":
      return [r.title ?? "Unknown game", r.system.toUpperCase(), r.serial, r.region].filter(Boolean).join(" · ");
    case "dvd-video":
      return "DVD-Video";
    case "blu-ray-video":
      return "Blu-ray video";
    case "video-cd":
      return r.super_vcd ? "Super Video CD" : "Video CD";
    case "pc":
      return "PC disc";
    case "data":
      return "Data disc";
  }
}

export default function App() {
  const [drives, setDrives] = useState<DriveInfo[]>([]);
  const [source, setSource] = useState("");
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void invoke<DriveInfo[]>("list_drives").then(setDrives);
  }, []);

  async function identify() {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      setReport(await invoke<Report>("identify_disc", { source: source.trim() || null }));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main>
      <h1>Spectra</h1>

      <section>
        <h2>Drives</h2>
        {drives.length === 0 ? (
          <p className="muted">No optical drive found. Plug one in, or identify an image below.</p>
        ) : (
          <ul>
            {drives.map((d) => (
              <li key={d.block ?? d.generic}>
                <button className="link" onClick={() => setSource(d.generic ?? d.block ?? "")}>
                  {d.block ?? d.generic}
                </button>{" "}
                {d.vendor} {d.model} <span className="muted">{d.revision}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          void identify();
        }}
      >
        <input
          value={source}
          onChange={(e) => setSource(e.currentTarget.value)}
          placeholder="/dev/sr0, or a path to a .cue / .bin / .iso (blank: first drive)"
        />
        <button type="submit" disabled={busy}>
          {busy ? "Reading…" : "Identify"}
        </button>
      </form>

      {error && <p className="error">{error}</p>}
      {report && (
        <section className="report">
          <h2>{describe(report)}</h2>
          <pre>{JSON.stringify(report, null, 2)}</pre>
        </section>
      )}
    </main>
  );
}
