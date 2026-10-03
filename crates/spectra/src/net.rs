//! The few web requests Spectra makes, through `curl`.
//!
//! `curl` is on every system Spectra runs on, and saves carrying an HTTP and
//! TLS stack of its own for a handful of requests per disc. Requests carry
//! nothing about the person asking: no cookies, no referrer, and a user agent
//! that names at most the app.

use std::process::{Command, Stdio};

pub enum Get {
    Found(Vec<u8>),
    /// The server says there is no such thing; asking again will not help.
    Missing,
    /// Offline, timed out, or the server is having a bad day.
    Failed,
}

/// Browsers' generic user agent, for image hosts.
pub const ANONYMOUS: &str = "Mozilla/5.0";
/// MusicBrainz asks clients to name themselves, and may turn away one that
/// does not.
pub const SPECTRA: &str = concat!("Spectra/", env!("CARGO_PKG_VERSION"));

pub fn get(url: &str, user_agent: &str) -> Get {
    let Ok(output) = Command::new("curl")
        .args(["--silent", "--location", "--max-time", "30"])
        .args(["--user-agent", user_agent])
        .args(["--write-out", "\n%{http_code}"])
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Get::Failed;
    };
    let mut body = output.stdout;
    // The status is the last line, after the body.
    let split = body.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let status = String::from_utf8_lossy(&body[split..]).trim().to_string();
    body.truncate(split);
    match status.as_str() {
        "200" if output.status.success() => Get::Found(body),
        "404" => Get::Missing,
        _ => Get::Failed,
    }
}
