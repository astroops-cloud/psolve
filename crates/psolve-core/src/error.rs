use std::fmt;

/// Why a frame did not solve. These are the machine-readable reason codes from
/// spec section 9 -- the caller distinguishes weather from misconfiguration by
/// this value, so the strings are a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonCode {
    CannotRead,
    UnsupportedFormat,
    NoStars,
    TooFewStars,
    ExtendedOnly,
    NoQuadMatch,
    LowConfidence,
    FovMismatch,
    /// No pointing hint was available at all -- neither `--hint` nor a usable
    /// header keyword (`OBJCTRA`/`OBJCTDEC` or `RA`/`DEC`). Distinct from
    /// `FovMismatch`: that means a hint WAS available but the field it
    /// implies does not match what was found, which is a data problem. This
    /// is a broken invocation (or an unsupported frame) wearing the wrong
    /// costume -- a caller branching on `reason` must not be told the field
    /// of view disagreed when in fact no hint was ever supplied.
    NoHint,
    IndexTooShallow,
    TimeBudgetExceeded,
}

impl ReasonCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasonCode::CannotRead => "CANNOT_READ",
            ReasonCode::UnsupportedFormat => "UNSUPPORTED_FORMAT",
            ReasonCode::NoStars => "NO_STARS",
            ReasonCode::TooFewStars => "TOO_FEW_STARS",
            ReasonCode::ExtendedOnly => "EXTENDED_ONLY",
            ReasonCode::NoQuadMatch => "NO_QUAD_MATCH",
            ReasonCode::LowConfidence => "LOW_CONFIDENCE",
            ReasonCode::FovMismatch => "FOV_MISMATCH",
            ReasonCode::NoHint => "NO_HINT",
            ReasonCode::IndexTooShallow => "INDEX_TOO_SHALLOW",
            ReasonCode::TimeBudgetExceeded => "TIME_BUDGET_EXCEEDED",
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A hard error: the input could not be processed at all. Distinct from a
/// ReasonCode, which says a well-formed frame did not yield a solution.
#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    Truncated { expected: usize, actual: usize },
    NoEndCard,
    MissingKeyword(&'static str),
    UnsupportedBitpix(i64),
    BadDimensions { nx: i64, ny: i64 },
    NotFits,
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::Truncated { expected, actual } => {
                write!(f, "fits truncated: need {expected} bytes, have {actual}")
            }
            SolveError::NoEndCard => write!(f, "fits header has no END card"),
            SolveError::MissingKeyword(k) => write!(f, "fits header missing {k}"),
            SolveError::UnsupportedBitpix(b) => write!(f, "unsupported BITPIX {b}"),
            SolveError::BadDimensions { nx, ny } => {
                write!(f, "implausible image dimensions {nx}x{ny}")
            }
            SolveError::NotFits => write!(f, "not a FITS file (no SIMPLE card)"),
        }
    }
}

impl std::error::Error for SolveError {}

impl SolveError {
    /// Every hard error maps to a reason code for the JSON output.
    pub fn reason(&self) -> ReasonCode {
        match self {
            SolveError::UnsupportedBitpix(_) => ReasonCode::UnsupportedFormat,
            _ => ReasonCode::CannotRead,
        }
    }
}
