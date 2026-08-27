//! Gaia DR3 ECSV parsing.
//!
//! The bulk files at cdn.gea.esac.esa.int are ECSV: 1,000 leading '#' comment
//! lines carrying a YAML header, then a CSV header row, then 152 columns of
//! data. Columns are located BY NAME -- never by hardcoded index -- because a
//! column order change would otherwise corrupt the whole catalogue silently.

use crate::error::IndexError;
use std::io::BufRead;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaiaRow {
    pub source_id: u64,
    pub ra: f64,
    pub dec: f64,
    pub mag: f32,
    pub pmra: f32,
    pub pmdec: f32,
}

/// Column NAMES to look for. Defaults are Gaia DR3's; override for any other
/// catalogue. The index format is not Gaia-specific and should not pretend
/// to be -- a Tycho-2 or Vizier export is a legitimate source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnNames {
    pub ra: String,
    pub dec: String,
    pub mag: String,
    pub pmra: String,
    pub pmdec: String,
    pub source_id: String,
}

impl Default for ColumnNames {
    fn default() -> Self {
        ColumnNames {
            ra: "ra".into(),
            dec: "dec".into(),
            mag: "phot_g_mean_mag".into(),
            pmra: "pmra".into(),
            pmdec: "pmdec".into(),
            source_id: "source_id".into(),
        }
    }
}

impl ColumnNames {
    /// Parse `"ra=RAJ2000,dec=DEJ2000,mag=Vmag"` over the defaults.
    /// An unknown key is an error rather than a silent no-op: a typo that is
    /// ignored produces an index built from the wrong column.
    pub fn with_overrides(spec: &str) -> Result<ColumnNames, IndexError> {
        let mut c = ColumnNames::default();
        for part in spec.split(',').filter(|p| !p.trim().is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| IndexError::BadColumnSpec(part.to_string()))?;
            let v = v.trim().to_string();
            match k.trim() {
                "ra" => c.ra = v,
                "dec" => c.dec = v,
                "mag" => c.mag = v,
                "pmra" => c.pmra = v,
                "pmdec" => c.pmdec = v,
                "source_id" => c.source_id = v,
                _ => return Err(IndexError::BadColumnSpec(part.to_string())),
            }
        }
        Ok(c)
    }
}

/// Which rows to keep. A fixed observatory never sees the whole sky, so a
/// declination cut removes stars that cannot appear in any frame it takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowFilter {
    pub max_mag: f32,
    pub min_dec: f64,
    pub max_dec: f64,
}

impl Default for RowFilter {
    fn default() -> Self {
        RowFilter { max_mag: f32::INFINITY, min_dec: -90.0, max_dec: 90.0 }
    }
}

impl RowFilter {
    pub fn validate(&self) -> Result<(), IndexError> {
        if !(-90.0..=90.0).contains(&self.min_dec) || !(-90.0..=90.0).contains(&self.max_dec) {
            return Err(IndexError::BadRange {
                what: "declination range",
                reason: format!("{}..{} is outside -90..90", self.min_dec, self.max_dec),
            });
        }
        if self.min_dec > self.max_dec {
            return Err(IndexError::BadRange {
                what: "declination range",
                reason: format!("min {} is above max {}", self.min_dec, self.max_dec),
            });
        }
        Ok(())
    }

    fn keeps(&self, row: &GaiaRow) -> bool {
        row.mag <= self.max_mag && row.dec >= self.min_dec && row.dec <= self.max_dec
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaiaColumns {
    /// usize::MAX when the catalogue has no source_id column.
    pub source_id: usize,
    pub ra: usize,
    pub dec: usize,
    pub pmra: usize,
    pub pmdec: usize,
    pub mag: usize,
    pub width: usize,
}

fn column_of(header: &[&str], name: &str) -> Result<usize, IndexError> {
    header
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| IndexError::MissingColumn(name.to_string()))
}

pub fn find_columns(header_line: &str, names: &ColumnNames) -> Result<GaiaColumns, IndexError> {
    let h: Vec<&str> = header_line.trim_end().split(',').collect();
    Ok(GaiaColumns {
        // Optional: reduced shards drop it, and nothing in the index needs it.
        // It is read when present only so tests can cross-check HEALPix
        // against Gaia's own source_id encoding.
        source_id: column_of(&h, &names.source_id).unwrap_or(usize::MAX),
        ra: column_of(&h, &names.ra)?,
        dec: column_of(&h, &names.dec)?,
        pmra: column_of(&h, &names.pmra)?,
        pmdec: column_of(&h, &names.pmdec)?,
        mag: column_of(&h, &names.mag)?,
        width: h.len(),
    })
}

/// Gaia DR3's bulk CSV export writes the literal string `null` for a missing
/// value and, empirically, never an empty field: sampling 2,000 real rows
/// from `GaiaSource_299573-302248` found 285 `pmra=null` and 0 `pmra=""`.
/// Treating `null` as corrupt (rather than as the same "not measured" case as
/// an empty field) means the first two-parameter source in a file -- which is
/// the overwhelming majority of rows -- aborts the entire rest of that file's
/// parse. `is_empty()` is kept alongside it rather than replaced: a
/// hand-built or non-Gaia CSV (e.g. via `--columns`) may still use a
/// genuinely empty field for "not measured", and both must mean the same
/// thing.
fn is_missing(s: &str) -> bool {
    s.is_empty() || s.eq_ignore_ascii_case("null")
}

/// Ok(None) means "no usable magnitude, skip this source".
pub fn parse_row(
    cols: &GaiaColumns,
    line: &str,
    line_no: u64,
) -> Result<Option<GaiaRow>, IndexError> {
    let f: Vec<&str> = line.trim_end().split(',').collect();
    let need = cols.width;
    if f.len() < need {
        return Err(IndexError::MalformedRow {
            line: line_no,
            reason: format!("expected {need} fields, found {}", f.len()),
        });
    }

    let mag_raw = f[cols.mag].trim();
    if is_missing(mag_raw) {
        return Ok(None);
    }

    let num = |i: usize, what: &str| -> Result<f64, IndexError> {
        f[i].trim().parse::<f64>().map_err(|e| IndexError::MalformedRow {
            line: line_no,
            reason: format!("{what}: {e}"),
        })
    };
    // Empty (or Gaia's literal "null", see is_missing above) means "not
    // measured" -- Gaia DR3 has ~340M two-parameter sources -- and is a
    // legitimate zero. Any other non-parseable value is corrupt input, and
    // must not be laundered into a plausible-looking 0.0.
    let opt = |i: usize, what: &str| -> Result<f32, IndexError> {
        let s = f[i].trim();
        if is_missing(s) {
            return Ok(0.0);
        }
        s.parse::<f32>().map_err(|e| IndexError::MalformedRow {
            line: line_no,
            reason: format!("{what}: {e}"),
        })
    };

    Ok(Some(GaiaRow {
        // source_id is advisory only -- nothing in the index needs it, it exists
        // so tests can cross-check HEALPix against Gaia's own encoding -- so a
        // garbage value is intentionally tolerated rather than failing the row.
        source_id: if cols.source_id == usize::MAX {
            0
        } else {
            f[cols.source_id].trim().parse().unwrap_or(0)
        },
        ra: num(cols.ra, "ra")?,
        dec: num(cols.dec, "dec")?,
        mag: mag_raw.parse::<f32>().map_err(|e| IndexError::MalformedRow {
            line: line_no,
            reason: format!("phot_g_mean_mag: {e}"),
        })?,
        pmra: opt(cols.pmra, "pmra")?,
        pmdec: opt(cols.pmdec, "pmdec")?,
    }))
}

/// Stream an ECSV/CSV file, invoking `f` for every row the filter keeps.
/// Returns the number of rows passed to `f`.
pub fn read_ecsv<R: BufRead, F: FnMut(GaiaRow)>(
    r: R,
    names: &ColumnNames,
    filter: &RowFilter,
    mut f: F,
) -> Result<u64, IndexError> {
    filter.validate()?;
    let mut cols: Option<GaiaColumns> = None;
    let mut kept = 0u64;
    for (i, line) in r.lines().enumerate() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        match &cols {
            None => cols = Some(find_columns(&line, names)?),
            Some(c) => {
                if let Some(row) = parse_row(c, &line, i as u64)? {
                    if filter.keeps(&row) {
                        f(row);
                        kept += 1;
                    }
                }
            }
        }
    }
    if cols.is_none() {
        return Err(IndexError::MalformedRow {
            line: 0,
            reason: "no header row found (file was empty or entirely comments)".into(),
        });
    }
    Ok(kept)
}
