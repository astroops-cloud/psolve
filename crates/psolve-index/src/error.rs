use std::fmt;

#[derive(Debug)]
pub enum IndexError {
    Io(std::io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    Truncated { expected: u64, actual: u64 },
    BadNside(u32),
    ChecksumMismatch,
    MissingColumn(String),
    BadColumnSpec(String),
    BadRange { what: &'static str, reason: String },
    MalformedRow { line: u64, reason: String },
    /// A `.psqidx`'s `star_index_fingerprint` does not match the paired
    /// `.psidx` it was opened against (`quad_reader::QuadIndex::open`). See
    /// `quad_format.rs`'s module doc, "Record layout: star-index references,
    /// not embedded positions" -- this is the tripwire against an
    /// accidental mispairing that would otherwise resolve every quad's star
    /// references into the wrong stars.
    FingerprintMismatch { expected: [u8; 8], found: [u8; 8] },
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "io: {e}"),
            IndexError::BadMagic => write!(f, "not a psolve index (bad magic)"),
            IndexError::UnsupportedVersion(v) => write!(f, "unsupported index version {v}"),
            IndexError::Truncated { expected, actual } => {
                write!(f, "index truncated: expected {expected} bytes, found {actual}")
            }
            IndexError::BadNside(n) => write!(f, "nside {n} is not a power of two in 1..=4096"),
            IndexError::ChecksumMismatch => write!(f, "index record checksum mismatch"),
            IndexError::MissingColumn(c) => write!(f, "catalogue csv missing column '{c}'"),
            IndexError::BadColumnSpec(s) => write!(
                f,
                "bad --columns entry '{s}' (expected key=name, key one of \
                 ra,dec,mag,pmra,pmdec,source_id)"
            ),
            IndexError::BadRange { what, reason } => write!(f, "bad {what}: {reason}"),
            IndexError::MalformedRow { line, reason } => {
                write!(f, "catalogue csv line {line}: {reason}")
            }
            IndexError::FingerprintMismatch { expected, found } => write!(
                f,
                "psqidx star_index_fingerprint {} does not match the paired .psidx \
                 (expected {})",
                hex8(found),
                hex8(expected)
            ),
        }
    }
}

fn hex8(b: &[u8; 8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e)
    }
}
