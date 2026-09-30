use std::fmt;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Scsi(#[from] ScsiError),
    /// The bytes are there but do not mean what they should.
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Unsupported(String),
}

impl Error {
    pub(crate) fn format(message: impl Into<String>) -> Self {
        Self::Format(message.into())
    }

    /// The drive says there is no disc in it.
    pub fn medium_not_present(&self) -> bool {
        matches!(self, Self::Scsi(e) if e.medium_not_present())
    }
}

/// A command the drive answered with CHECK CONDITION, and the sense data that
/// says why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiError {
    pub command: &'static str,
    pub key: u8,
    pub asc: u8,
    pub ascq: u8,
}

impl ScsiError {
    pub fn medium_not_present(&self) -> bool {
        self.key == 0x02 && self.asc == 0x3a
    }

    /// Not ready for some reason other than an empty tray: usually spinning up.
    pub fn becoming_ready(&self) -> bool {
        self.key == 0x02 && !self.medium_not_present()
    }

    /// The drive reporting that something changed - a disc went in, a reset.
    /// The command itself was not run and can be sent again.
    pub fn unit_attention(&self) -> bool {
        self.key == 0x06
    }
}

const SENSE_KEYS: [&str; 16] = [
    "no sense",
    "recovered error",
    "not ready",
    "medium error",
    "hardware error",
    "illegal request",
    "unit attention",
    "data protect",
    "blank check",
    "vendor specific",
    "copy aborted",
    "aborted command",
    "equal",
    "volume overflow",
    "miscompare",
    "reserved",
];

impl fmt::Display for ScsiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match (self.key, self.asc) {
            (0x02, 0x3a) => "no disc in the drive".to_string(),
            (0x02, 0x04) => "the drive is still spinning up".to_string(),
            (0x03, 0x11) => "unreadable sector".to_string(),
            (0x05, 0x20) => "the drive does not support that command".to_string(),
            (0x05, 0x21) => "read past the end of the disc".to_string(),
            (0x05, 0x64) => "wrong sector type for this track".to_string(),
            (0x05, 0x6f) => "the disc is copy protected and the drive will not read it".to_string(),
            _ => format!(
                "{} (ASC {:02x}/{:02x})",
                SENSE_KEYS[usize::from(self.key & 0x0f)],
                self.asc,
                self.ascq
            ),
        };
        write!(f, "{}: {what}", self.command)
    }
}

impl std::error::Error for ScsiError {}
