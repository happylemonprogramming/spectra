//! An optical drive, spoken to with raw SCSI MMC commands over Linux SG_IO.
//!
//! This replaces Rainbow Player's WebUSB Bulk-Only Transport (`bot.ts`): the
//! command blocks are the same, but the kernel carries them, so any drive
//! works - SATA or USB, BOT or UAS - and `/dev/sr0` stays where it is. The
//! kernel also collects sense data after a failed command, which BOT made us
//! do by hand.
//!
//! Opened read-only, the kernel only passes commands it considers safe
//! (reads, TOC, INQUIRY, GET CONFIGURATION and the like), which is all
//! identification needs. Commands that change state - eject, read speed -
//! need the device opened for writing, which `uaccess` normally allows.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::disc::{Disc, Media, RAW_SECTOR, SECTOR, Toc, Track};
use crate::{Error, Result, ScsiError};

/// Sectors per READ(12): a 64 KB transfer, which every USB bridge accepts.
const SECTORS_PER_READ: u32 = 32;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// A drive as sysfs describes it, before anything is opened.
#[derive(Debug, Clone, Serialize)]
pub struct DriveInfo {
    /// `/dev/sgN`, if the sg module is loaded.
    pub generic: Option<PathBuf>,
    /// `/dev/srN`.
    pub block: Option<PathBuf>,
    pub vendor: String,
    pub model: String,
    pub revision: String,
}

impl DriveInfo {
    /// The node to open: sg if there is one, the block device otherwise.
    pub fn path(&self) -> Option<&Path> {
        self.generic.as_deref().or(self.block.as_deref())
    }
}

/// Every optical drive the kernel knows about (SCSI peripheral type 5).
pub fn list() -> Vec<DriveInfo> {
    let read = |p: &Path| {
        std::fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let child = |dir: &Path, class: &str| {
        std::fs::read_dir(dir.join(class))
            .ok()?
            .flatten()
            .next()
            .map(|e| Path::new("/dev").join(e.file_name()))
    };
    let Ok(devices) = std::fs::read_dir("/sys/bus/scsi/devices") else {
        return Vec::new();
    };
    let mut drives: Vec<DriveInfo> = devices
        .flatten()
        .map(|e| e.path())
        .filter(|dir| read(&dir.join("type")) == "5")
        .map(|dir| DriveInfo {
            generic: child(&dir, "scsi_generic"),
            block: child(&dir, "block"),
            vendor: read(&dir.join("vendor")),
            model: read(&dir.join("model")),
            revision: read(&dir.join("rev")),
        })
        .collect();
    drives.sort_by(|a, b| a.block.cmp(&b.block));
    drives
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MediaStatus {
    pub present: bool,
    pub tray_open: bool,
    /// 0 none, 1 eject requested, 2 new media, 3 removal, 4 changed.
    pub event: u8,
}

pub struct Drive {
    file: File,
    path: PathBuf,
    writable: bool,
}

enum Direction {
    None,
    In(usize),
}

// <scsi/sg.h>
const SG_IO: libc::c_ulong = 0x2285;
const SG_DXFER_NONE: libc::c_int = -1;
const SG_DXFER_FROM_DEV: libc::c_int = -3;
const CHECK_CONDITION: u8 = 0x02;

#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,
    dxfer_direction: libc::c_int,
    cmd_len: libc::c_uchar,
    mx_sb_len: libc::c_uchar,
    iovec_count: libc::c_ushort,
    dxfer_len: libc::c_uint,
    dxferp: *mut libc::c_void,
    cmdp: *const libc::c_uchar,
    sbp: *mut libc::c_uchar,
    timeout: libc::c_uint,
    flags: libc::c_uint,
    pack_id: libc::c_int,
    usr_ptr: *mut libc::c_void,
    status: libc::c_uchar,
    masked_status: libc::c_uchar,
    msg_status: libc::c_uchar,
    sb_len_wr: libc::c_uchar,
    host_status: libc::c_ushort,
    driver_status: libc::c_ushort,
    resid: libc::c_int,
    duration: libc::c_uint,
    info: libc::c_uint,
}

impl Drive {
    /// Open a drive by device node (`/dev/sg1` or `/dev/sr0`).
    ///
    /// Non-blocking, so an empty drive opens instead of failing, and the
    /// kernel does not lock the tray as it would for a mount.
    pub fn open(path: &Path) -> Result<Self> {
        let open = |write: bool| {
            OpenOptions::new()
                .read(true)
                .write(write)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
        };
        let (file, writable) = match open(true) {
            Ok(file) => (file, true),
            Err(_) => (open(false)?, false),
        };
        Ok(Self {
            file,
            path: path.to_path_buf(),
            writable,
        })
    }

    /// The first drive that opens.
    pub fn open_first() -> Result<Self> {
        let drives = list();
        if drives.is_empty() {
            return Err(Error::Unsupported("no optical drive found".into()));
        }
        let mut last = None;
        for drive in &drives {
            for path in [&drive.generic, &drive.block].into_iter().flatten() {
                match Self::open(path) {
                    Ok(d) => return Ok(d),
                    Err(e) => last = Some(e),
                }
            }
        }
        Err(last.expect("at least one drive was tried"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opened for writing, so state-changing commands (eject) are allowed.
    pub fn writable(&self) -> bool {
        self.writable
    }

    fn command(
        &self,
        name: &'static str,
        cdb: &[u8],
        direction: Direction,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        // A unit attention reports something that already happened - a disc
        // going in, a bus reset - and the command was not run. Say it again.
        for _ in 0..3 {
            match self.command_once(name, cdb, &direction, timeout) {
                Err(Error::Scsi(e)) if e.unit_attention() => continue,
                other => return other,
            }
        }
        self.command_once(name, cdb, &direction, timeout)
    }

    fn command_once(
        &self,
        name: &'static str,
        cdb: &[u8],
        direction: &Direction,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let (dxfer_direction, length) = match direction {
            Direction::None => (SG_DXFER_NONE, 0),
            Direction::In(n) => (SG_DXFER_FROM_DEV, *n),
        };
        let mut data = vec![0u8; length];
        let mut sense = [0u8; 64];
        let mut hdr = SgIoHdr {
            interface_id: libc::c_int::from(b'S'),
            dxfer_direction,
            cmd_len: cdb.len() as u8,
            mx_sb_len: sense.len() as u8,
            iovec_count: 0,
            dxfer_len: length as u32,
            dxferp: data.as_mut_ptr().cast(),
            cmdp: cdb.as_ptr(),
            sbp: sense.as_mut_ptr(),
            timeout: timeout.as_millis() as u32,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };
        // SAFETY: every pointer in `hdr` refers to a live buffer of the
        // length it claims, for the duration of the call.
        let rc = unsafe { libc::ioctl(self.file.as_raw_fd(), SG_IO as _, &mut hdr) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if hdr.status == CHECK_CONDITION || hdr.sb_len_wr > 0 {
            let e = parse_sense(name, &sense[..usize::from(hdr.sb_len_wr)]);
            // Key 0 is no error and key 1 an error the drive recovered from.
            if e.key > 1 {
                return Err(e.into());
            }
        } else if hdr.status != 0 || hdr.host_status != 0 {
            return Err(Error::Io(std::io::Error::other(format!(
                "{name}: transport failed (status {:#x}, host {:#x}, driver {:#x})",
                hdr.status, hdr.host_status, hdr.driver_status
            ))));
        }
        data.truncate(length.saturating_sub(hdr.resid.max(0) as usize));
        Ok(data)
    }

    /// Vendor, model and firmware revision, as the drive reports them.
    pub fn inquiry(&self) -> Result<(String, String, String)> {
        let reply = self.command(
            "inquiry",
            &[0x12, 0, 0, 0, 36, 0],
            Direction::In(36),
            STATUS_TIMEOUT,
        )?;
        let field = |from: usize, to: usize| {
            reply
                .get(from..to.min(reply.len()))
                .map(|b| String::from_utf8_lossy(b).trim().to_string())
                .unwrap_or_default()
        };
        Ok((field(8, 16), field(16, 32), field(32, 36)))
    }

    pub fn test_unit_ready(&self) -> Result<()> {
        self.command("test unit ready", &[0; 6], Direction::None, STATUS_TIMEOUT)
            .map(drop)
    }

    /// Block until the drive will answer reads, or until it says it never will.
    ///
    /// A drive reports media several seconds before it can be read - it
    /// still has to spin up and take in the lead-in - and on a slim USB drive
    /// that runs to seven seconds or more. An empty tray says so with a
    /// different sense code and returns at once.
    pub fn wait_until_ready(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.test_unit_ready() {
                Err(Error::Scsi(e)) if e.becoming_ready() && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(400));
                }
                other => return other,
            }
        }
    }

    /// GET CONFIGURATION's current profile: what the drive believes is in it.
    pub fn profile(&self) -> Result<u16> {
        let reply = self.command(
            "get configuration",
            &[0x46, 0x02, 0, 0, 0, 0, 0, 0, 8, 0],
            Direction::In(8),
            STATUS_TIMEOUT,
        )?;
        Ok(if reply.len() >= 8 {
            u16::from_be_bytes([reply[6], reply[7]])
        } else {
            0
        })
    }

    /// READ TOC/PMA/ATIP, format 0: a four-byte header, then an eight-byte
    /// descriptor per track and one for the lead-out (track 0xAA).
    pub fn read_toc(&self) -> Result<Toc> {
        const ALLOCATION: usize = 4 + 8 * 100;
        let reply = self.command(
            "read TOC",
            &[
                0x43,
                0,
                0,
                0,
                0,
                0,
                1,
                (ALLOCATION >> 8) as u8,
                ALLOCATION as u8,
                0,
            ],
            Direction::In(ALLOCATION),
            READ_TIMEOUT,
        )?;
        parse_toc(&reply)
    }

    /// READ CAPACITY: the last addressable sector, plus one.
    pub fn read_capacity(&self) -> Result<u32> {
        let reply = self.command(
            "read capacity",
            &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            Direction::In(8),
            READ_TIMEOUT,
        )?;
        if reply.len() < 4 {
            return Err(Error::format("short READ CAPACITY reply"));
        }
        Ok(u32::from_be_bytes([reply[0], reply[1], reply[2], reply[3]]) + 1)
    }

    /// GET EVENT/STATUS NOTIFICATION for the media class: whether the disc
    /// changed, without reading it. Cheap enough to poll.
    pub fn media_status(&self) -> Result<MediaStatus> {
        let reply = self.command(
            "get media status",
            &[0x4a, 0x01, 0, 0, 0x10, 0, 0, 0, 8, 0],
            Direction::In(8),
            STATUS_TIMEOUT,
        )?;
        if reply.len() < 6 {
            return Ok(MediaStatus {
                present: false,
                tray_open: false,
                event: 0,
            });
        }
        // NEA: no event available, so the event byte means nothing.
        let no_event = reply[2] & 0x80 != 0;
        Ok(MediaStatus {
            present: reply[5] & 0x02 != 0,
            tray_open: reply[5] & 0x01 != 0,
            event: if no_event { 0 } else { reply[4] & 0x0f },
        })
    }

    /// Unlock the tray, then open it. A locked drive answers the eject with an
    /// error and only spins down, which looks like a stuck tray.
    pub fn eject(&self) -> Result<()> {
        self.command(
            "allow medium removal",
            &[0x1e, 0, 0, 0, 0, 0],
            Direction::None,
            STATUS_TIMEOUT,
        )?;
        self.command(
            "eject",
            &[0x1b, 0, 0, 0, 0x02, 0],
            Direction::None,
            READ_TIMEOUT,
        )
        .map(drop)
    }

    /// READ CD: sectors as they are on the disc, 2352 bytes each. A data
    /// sector comes with its sync pattern, header and error correction; an
    /// audio one is all samples, little-endian, as a `.bin` keeps them.
    pub fn read_raw(&self, lba: u32, count: u32, audio: bool) -> Result<Vec<u8>> {
        let [a, b, c, d] = lba.to_be_bytes();
        let [_, e, f, g] = count.to_be_bytes();
        let (sector_type, fields) = if audio {
            // CD-DA; user data, which for audio is the whole sector.
            (0x04, 0x10)
        } else {
            // Any type; sync, all headers, user data, EDC/ECC.
            (0x00, 0xf8)
        };
        self.command(
            "read CD",
            &[0xbe, sector_type, a, b, c, d, e, f, g, fields, 0, 0],
            Direction::In(count as usize * RAW_SECTOR),
            READ_TIMEOUT,
        )
    }

    fn read12(&self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let [a, b, c, d] = lba.to_be_bytes();
        let [e, f, g, h] = count.to_be_bytes();
        self.command(
            "read",
            &[0xa8, 0, a, b, c, d, e, f, g, h, 0, 0],
            Direction::In(count as usize * SECTOR),
            READ_TIMEOUT,
        )
    }
}

fn parse_sense(command: &'static str, sense: &[u8]) -> ScsiError {
    let at = |i: usize| sense.get(i).copied().unwrap_or(0);
    // Fixed format (0x70/0x71) keeps key, ASC and ASCQ at 2, 12 and 13;
    // descriptor format (0x72/0x73) at 1, 2 and 3.
    let (key, asc, ascq) = if at(0) & 0x7f >= 0x72 {
        (at(1) & 0x0f, at(2), at(3))
    } else {
        (at(2) & 0x0f, at(12), at(13))
    };
    ScsiError {
        command,
        key,
        asc,
        ascq,
    }
}

pub(crate) fn parse_toc(reply: &[u8]) -> Result<Toc> {
    if reply.len() < 4 {
        return Err(Error::format(
            "the drive returned an empty table of contents",
        ));
    }
    let length = (usize::from(u16::from_be_bytes([reply[0], reply[1]])) + 2).min(reply.len());
    let mut tracks = Vec::new();
    let mut leadout = None;
    for d in reply[4..length].as_chunks::<8>().0 {
        let lba = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
        match d[2] {
            0xaa => leadout = Some(lba),
            number => tracks.push(Track {
                number,
                lba,
                data: d[1] & 0x04 != 0,
            }),
        }
    }
    let leadout = leadout.ok_or_else(|| Error::format("the table of contents has no lead-out"))?;
    if tracks.is_empty() {
        return Err(Error::format("the table of contents has no tracks"));
    }
    Ok(Toc {
        first: reply[2],
        last: reply[3],
        tracks,
        leadout,
    })
}

impl Disc for Drive {
    fn describe(&self) -> String {
        match self.inquiry() {
            Ok((vendor, model, revision)) => {
                format!("{} ({vendor} {model} {revision})", self.path.display())
            }
            Err(_) => self.path.display().to_string(),
        }
    }

    fn media(&mut self) -> Result<Media> {
        Ok(Media::from_profile(self.profile()?))
    }

    fn toc(&mut self) -> Result<Option<Toc>> {
        // DVDs and Blu-rays report a nominal one-track TOC; it says nothing.
        match self.media()? {
            Media::Dvd | Media::BluRay | Media::HdDvd => Ok(None),
            Media::Cd | Media::Unknown => self.read_toc().map(Some),
        }
    }

    fn read(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(count as usize * SECTOR);
        let mut at = lba;
        while at < lba + count {
            let n = SECTORS_PER_READ.min(lba + count - at);
            out.extend(self.read12(at, n)?);
            at += n;
        }
        Ok(out)
    }

    fn capacity(&mut self) -> Result<Option<u32>> {
        self.read_capacity().map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_with_enhanced_data_track() {
        let mut reply = vec![0, 0, 1, 3];
        for (control, track, lba) in [
            (0x10u8, 1u8, 0u32),
            (0x10, 2, 15_000),
            (0x14, 3, 41_400),
            (0x14, 0xaa, 60_000),
        ] {
            reply.extend([0, control, track, 0]);
            reply.extend(lba.to_be_bytes());
        }
        let length = (reply.len() - 2) as u16;
        reply[..2].copy_from_slice(&length.to_be_bytes());

        let toc = parse_toc(&reply).unwrap();
        assert_eq!((toc.first, toc.last, toc.leadout), (1, 3, 60_000));
        assert_eq!(
            toc.tracks.iter().map(|t| t.data).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(toc.enhanced_data_track().map(|t| t.number), Some(3));
        assert!(toc.boot_track().is_none());
    }

    #[test]
    fn sense_formats() {
        let mut fixed = [0u8; 18];
        fixed[0] = 0x70;
        fixed[2] = 0x02;
        fixed[12] = 0x3a;
        assert!(parse_sense("x", &fixed).medium_not_present());
        let descriptor = [0x72, 0x02, 0x3a, 0x00];
        assert!(parse_sense("x", &descriptor).medium_not_present());
    }
}
