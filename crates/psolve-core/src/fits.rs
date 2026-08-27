//! FITS reading, from bytes only.
//!
//! A FITS header is a sequence of 80-byte fixed-width cards, packed into
//! 2880-byte blocks, terminated by an `END` card and padded to a block
//! boundary. This is untrusted input: every accessor returns an Option or a
//! Result, and nothing here can panic on malformed bytes.

use crate::error::SolveError;

const CARD: usize = 80;
const BLOCK: usize = 2880;
/// A header longer than this is not a header we want to walk.
const MAX_BLOCKS: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct FitsHeader {
    /// (key, raw value) in file order. Keys may repeat -- COMMENT, HISTORY --
    /// so this is a list, not a map, and `get` returns the first match.
    pub cards: Vec<(String, String)>,
    /// Byte offset of the data unit: the first block after the END card.
    pub data_offset: usize,
}

/// Strip a FITS value of its trailing comment, surrounding quotes and padding.
fn clean_value(raw: &str) -> String {
    let v = raw.trim();
    // A quoted string may legitimately contain '/', so handle quotes first.
    if let Some(rest) = v.strip_prefix('\'') {
        if let Some(end) = rest.find('\'') {
            return rest[..end].trim().to_string();
        }
        return rest.trim().to_string();
    }
    match v.split_once('/') {
        Some((before, _)) => before.trim().to_string(),
        None => v.to_string(),
    }
}

impl FitsHeader {
    pub fn parse(bytes: &[u8]) -> Result<FitsHeader, SolveError> {
        if bytes.len() < BLOCK {
            return Err(SolveError::Truncated { expected: BLOCK, actual: bytes.len() });
        }
        if &bytes[..6] != b"SIMPLE" {
            return Err(SolveError::NotFits);
        }

        let mut cards = Vec::new();
        let mut off = 0usize;
        for _ in 0..MAX_BLOCKS {
            if off >= bytes.len() {
                break;
            }
            if off + BLOCK > bytes.len() {
                return Err(SolveError::Truncated {
                    expected: off + BLOCK,
                    actual: bytes.len(),
                });
            }
            for i in 0..(BLOCK / CARD) {
                let s = off + i * CARD;
                let card = &bytes[s..s + CARD];
                let key = String::from_utf8_lossy(&card[..8]).trim().to_string();
                if key == "END" {
                    return Ok(FitsHeader { cards, data_offset: off + BLOCK });
                }
                if card.len() > 9 && card[8] == b'=' {
                    let raw = String::from_utf8_lossy(&card[9..]).to_string();
                    cards.push((key, clean_value(&raw)));
                }
            }
            off += BLOCK;
        }
        Err(SolveError::NoEndCard)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.cards.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        // Older FITS uses D as the exponent marker; Rust wants E.
        let v = self.get(key)?.replace(['D', 'd'], "E");
        // A non-finite value is corrupt, not a number. Returning Some here
        // would let `int()`'s cast saturate NaN to 0 and infinity to i64::MAX,
        // handing a caller a plausible-looking value for a broken card.
        v.parse::<f64>().ok().filter(|x| x.is_finite())
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.num(key).map(|v| v as i64)
    }
}

/// A decoded frame in f32. The pipeline subtracts a background surface almost
/// immediately, so integer types stop helping at that point; one conversion
/// here is simpler than a type parameter threaded through five modules.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub nx: usize,
    pub ny: usize,
    pub px: Vec<f32>,
    /// 2 when a CFA mosaic was superpixel-binned, else 1. The caller needs this
    /// because the pixel scale doubles with it.
    pub binned: u32,
}

/// Anything larger than this is not a frame we are going to solve, and
/// checking before allocating stops a malformed header exhausting memory.
const MAX_PIXELS: i64 = 600_000_000;

pub fn decode(bytes: &[u8], h: &FitsHeader) -> Result<Image, SolveError> {
    // Match on the i64 directly: `4294967312i64 as i32 == 16`, so casting
    // before the match let a corrupt BITPIX decode as if it were valid
    // 16-bit data instead of failing. This is the last place that could
    // happen -- `num()` was already hardened against a NaN/overflow card,
    // and this closes the equivalent hole in the cast that followed it.
    let bitpix = h.int("BITPIX").ok_or(SolveError::MissingKeyword("BITPIX"))?;
    let nx = h.int("NAXIS1").ok_or(SolveError::MissingKeyword("NAXIS1"))?;
    let ny = h.int("NAXIS2").ok_or(SolveError::MissingKeyword("NAXIS2"))?;
    if nx <= 0 || ny <= 0 || nx.saturating_mul(ny) > MAX_PIXELS {
        return Err(SolveError::BadDimensions { nx, ny });
    }
    let (nx, ny) = (nx as usize, ny as usize);

    let width = match bitpix {
        8 => 1,
        16 => 2,
        32 | -32 => 4,
        other => return Err(SolveError::UnsupportedBitpix(other)),
    };
    let need = h.data_offset + nx * ny * width;
    if bytes.len() < need {
        return Err(SolveError::Truncated { expected: need, actual: bytes.len() });
    }

    let bzero = h.num("BZERO").unwrap_or(0.0) as f32;
    let bscale = {
        let s = h.num("BSCALE").unwrap_or(1.0) as f32;
        if s == 0.0 { 1.0 } else { s }
    };
    let raw = &bytes[h.data_offset..need];

    let mut px = vec![0f32; nx * ny];
    for (i, p) in px.iter_mut().enumerate() {
        let o = i * width;
        let v: f32 = match bitpix {
            8 => raw[o] as f32,
            16 => i16::from_be_bytes([raw[o], raw[o + 1]]) as f32,
            32 => i32::from_be_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]) as f32,
            -32 => f32::from_be_bytes([raw[o], raw[o + 1], raw[o + 2], raw[o + 3]]),
            _ => unreachable!("bitpix was validated above"),
        };
        *p = v * bscale + bzero;
    }

    // A one-shot-colour frame is a Bayer mosaic: neighbouring pixels are
    // different filters, so star shapes are meaningless until it is binned.
    if h.get("BAYERPAT").is_some() && nx >= 2 && ny >= 2 {
        let (bx, by) = (nx / 2, ny / 2);
        let mut out = vec![0f32; bx * by];
        for y in 0..by {
            for x in 0..bx {
                let a = px[2 * y * nx + 2 * x];
                let b = px[2 * y * nx + 2 * x + 1];
                let c = px[(2 * y + 1) * nx + 2 * x];
                let d = px[(2 * y + 1) * nx + 2 * x + 1];
                out[y * bx + x] = (a + b + c + d) * 0.25;
            }
        }
        return Ok(Image { nx: bx, ny: by, px: out, binned: 2 });
    }

    Ok(Image { nx, ny, px, binned: 1 })
}

/// Arcseconds per pixel from the optics keywords, accounting for binning.
/// 206.265 * pixel_size_um * binning / focal_length_mm.
pub fn pixel_scale_arcsec(h: &FitsHeader) -> Option<f64> {
    let focal = h.num("FOCALLEN")?;
    let pix = h.num("XPIXSZ").or_else(|| h.num("YPIXSZ"))?;
    if focal <= 0.0 || pix <= 0.0 {
        return None;
    }
    let binning = h.num("XBINNING").filter(|b| *b >= 1.0).unwrap_or(1.0);
    Some(206.265 * pix * binning / focal)
}

/// Field HEIGHT in degrees -- the quantity ASTAP's -fov wants, and the one the
/// hint uses to size its search. ASTAP's own auto-detect mis-guesses this rig
/// badly (about 9.5 degrees against a true 1.48), which is why it is computed
/// from the header whenever the optics keywords allow.
pub fn field_height_deg(h: &FitsHeader) -> Option<f64> {
    let scale = pixel_scale_arcsec(h)?;
    let ny = h.num("NAXIS2")?;
    if ny <= 0.0 {
        return None;
    }
    Some(ny * scale / 3600.0)
}

/// Field WIDTH in degrees -- the horizontal counterpart to
/// [`field_height_deg`], computed the same way from `NAXIS1`. Mirrors that
/// function's keyword handling and failure behaviour exactly.
pub fn field_width_deg(h: &FitsHeader) -> Option<f64> {
    let scale = pixel_scale_arcsec(h)?;
    let nx = h.num("NAXIS1")?;
    if nx <= 0.0 {
        return None;
    }
    Some(nx * scale / 3600.0)
}

/// The superpixel-binning factor `decode` will apply, without doing the full
/// pixel decode. A CFA (`BAYERPAT`) frame is binned 2x2; anything else is
/// unbinned. A caller that must interpret a pixel-space quantity (e.g. an
/// explicit `--scale`) before it has decoded the image uses this to match
/// what `decode` will actually do to the grid.
pub fn binning_factor(h: &FitsHeader) -> u32 {
    let nx = h.int("NAXIS1").unwrap_or(0);
    let ny = h.int("NAXIS2").unwrap_or(0);
    if h.get("BAYERPAT").is_some() && nx >= 2 && ny >= 2 { 2 } else { 1 }
}

fn sexagesimal(s: &str) -> Option<(f64, f64, f64, f64)> {
    let t = s.trim();
    let sign = if t.starts_with('-') { -1.0 } else { 1.0 };
    let parts: Vec<f64> = t
        .trim_start_matches(['+', '-'])
        .split(|c: char| c == ':' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f64>().ok())
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some((
        sign,
        parts[0],
        *parts.get(1).unwrap_or(&0.0),
        *parts.get(2).unwrap_or(&0.0),
    ))
}

/// The mount's commanded pointing, used to seed the search. These are NOT a
/// solution -- they are where the telescope was told to look.
///
/// Tries the sexagesimal `OBJCTRA`/`OBJCTDEC` pair first, and only when that
/// pair is entirely absent or unusable falls back to the decimal-degree
/// `RA`/`DEC` pair DWARFIII frames carry instead -- a fallback, not a
/// replacement, so a frame carrying both keeps using the sexagesimal one.
pub fn hint_radec(h: &FitsHeader) -> Option<(f64, f64)> {
    sexagesimal_hint(h).or_else(|| decimal_hint(h))
}

/// `OBJCTRA`/`OBJCTDEC`, sexagesimal, hours/degrees-minutes-seconds.
fn sexagesimal_hint(h: &FitsHeader) -> Option<(f64, f64)> {
    let (_, rh, rm, rs) = sexagesimal(h.get("OBJCTRA")?)?;
    let (sign, dd, dm, ds) = sexagesimal(h.get("OBJCTDEC")?)?;
    let ra = (rh + rm / 60.0 + rs / 3600.0) * 15.0;
    let dec = sign * (dd + dm / 60.0 + ds / 3600.0);
    if !(0.0..=360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) {
        return None;
    }
    Some((ra, dec))
}

/// `RA`/`DEC`, decimal DEGREES -- what DWARFIII frames carry instead of
/// `OBJCTRA`/`OBJCTDEC`. Verified empirically, not derived: six real DWARFIII
/// frames ASTAP solved carry `RA = 83.82208`, `DEC = -5.39111` against
/// ASTAP's own solved `ra = 83.861`, `dec = -5.397` -- a ~2.3' residual,
/// which is ordinary commanded-vs-solved pointing error, not a unit
/// mismatch. Do not confuse this with ASTAP's `-ra` *command-line* flag,
/// which is in HOURS -- that is a different value with the same name, and
/// conflating them is a 15x error that surfaces only as `NO_QUAD_MATCH`.
///
/// `h.num()` already rejects a non-finite card (NaN, an overflowing
/// exponent) as absent, so that guard does not need repeating here.
fn decimal_hint(h: &FitsHeader) -> Option<(f64, f64)> {
    let ra = h.num("RA")?;
    let dec = h.num("DEC")?;
    if !(0.0..=360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) {
        return None;
    }
    // At least one real capture (an UnknownOBJECT frame) carries
    // `DEC = -90.` as an unset-value sentinel, not a genuine pointing at the
    // pole. A hint exactly at the pole is legal in principle, but in
    // practice a header that says "the mount was pointed at the exact
    // celestial pole" is far more likely to mean "no target was ever set"
    // than to be true -- and a wrong hint here is worse than no hint: it
    // seeds a catalogue search around the wrong point instead of correctly
    // reporting NO_HINT. `+90.0` has no observed sentinel evidence (a
    // northern circumpolar target legitimately points close to it), so only
    // the observed `-90.0` sentinel is rejected, deliberately asymmetrically.
    if dec == -90.0 {
        return None;
    }
    Some((ra, dec))
}

/// DATE-OBS as a decimal year, for proper-motion correction.
///
/// DATE-OBS carries no timezone suffix and N.I.N.A. writes it in UTC while the
/// FILENAME is local. Treating it as local time is a 12-hour error that has
/// bitten this project twice -- once turning a 1.26 degree pointing check into
/// a reported 47 degrees. It is always UTC here.
pub fn epoch_years(h: &FitsHeader) -> Option<f64> {
    let s = h.get("DATE-OBS")?;
    let date = s.split('T').next()?;
    let mut it = date.split('-');
    let y: f64 = it.next()?.parse().ok()?;
    let mo: f64 = it.next().unwrap_or("1").parse().unwrap_or(1.0);
    let d: f64 = it.next().unwrap_or("1").parse().unwrap_or(1.0);
    if !(1900.0..=2200.0).contains(&y) {
        return None;
    }
    // Day-of-year to a couple of days is far more precision than proper motion
    // needs over a decade; a full calendar is not worth the code.
    let doy = (mo - 1.0) * 30.4375 + (d - 1.0);
    Some(y + doy / 365.25)
}

#[cfg(test)]
mod header_tests {
    use super::*;
    use crate::SolveError;

    /// Build a synthetic FITS header: cards padded to 80, blocks to 2880.
    fn header_bytes(cards: &[&str]) -> Vec<u8> {
        let mut s = String::new();
        for c in cards {
            s.push_str(&format!("{c:<80}"));
        }
        s.push_str(&format!("{:<80}", "END"));
        while !s.len().is_multiple_of(2880) {
            s.push(' ');
        }
        s.into_bytes()
    }

    #[test]
    fn parses_a_minimal_header() {
        let b = header_bytes(&[
            "SIMPLE  =                    T",
            "BITPIX  =                   16",
            "NAXIS   =                    2",
            "NAXIS1  =                 3840",
            "NAXIS2  =                 2160",
        ]);
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(h.int("BITPIX"), Some(16));
        assert_eq!(h.int("NAXIS1"), Some(3840));
        assert_eq!(h.data_offset, 2880);
    }

    #[test]
    fn strips_quotes_and_comments_from_string_values() {
        let b = header_bytes(&[
            "SIMPLE  =                    T",
            "FILTER  = 'H'                  / Active filter name",
            "OBJCTRA = '18 18 48'           / [H M S] RA of imaged object",
        ]);
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(h.get("FILTER"), Some("H"));
        assert_eq!(h.get("OBJCTRA"), Some("18 18 48"));
    }

    #[test]
    fn parses_floats_in_fits_exponent_form() {
        // FITS permits D as the exponent marker; real files use E.
        // Older FITS files use D as the exponent marker. This is the whole
        // reason num() rewrites it, and nothing tested it.
        let b = header_bytes(&[
            "SIMPLE  =                    T",
            "CRVAL1  =  2.746890869201E+002",
            "CDELT1  = -6.8141055409E-004",
            "OLDSTYLE=  2.746890869201D+002",
            "LOWERD  =  1.5d+003",
        ]);
        let h = FitsHeader::parse(&b).unwrap();
        assert!((h.num("CRVAL1").unwrap() - 274.6890869201).abs() < 1e-9);
        assert!((h.num("CDELT1").unwrap() + 6.8141055409e-4).abs() < 1e-15);
        assert!((h.num("OLDSTYLE").unwrap() - 274.6890869201).abs() < 1e-9);
        assert!((h.num("LOWERD").unwrap() - 1500.0).abs() < 1e-9);
    }

    #[test]
    fn a_header_spanning_several_blocks_finds_its_end() {
        // 100 cards is more than one 36-card block.
        let mut cards: Vec<String> = Vec::new();
        cards.push("SIMPLE  =                    T".to_string());
        for i in 0..100 {
            cards.push(format!("COMMENT filler card number {i}"));
        }
        cards.push("BITPIX  =                   16".to_string());
        let refs: Vec<&str> = cards.iter().map(|s| s.as_str()).collect();
        let b = header_bytes(&refs);
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(h.int("BITPIX"), Some(16));
        assert_eq!(h.data_offset % 2880, 0);
        assert!(h.data_offset >= 2880 * 3);
    }

    #[test]
    fn blank_and_malformed_cards_do_not_derail_the_parse() {
        let b = header_bytes(&[
            "SIMPLE  =                    T",
            "",
            "GARBAGE WITHOUT AN EQUALS SIGN",
            "BITPIX  =                   16",
        ]);
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(h.int("BITPIX"), Some(16));
    }

    #[test]
    fn a_header_with_no_end_card_is_an_error_not_a_hang() {
        let mut b = vec![b' '; 2880 * 2];
        b[..30].copy_from_slice(b"SIMPLE  =                    T");
        assert_eq!(FitsHeader::parse(&b), Err(SolveError::NoEndCard));
    }

    #[test]
    fn a_non_fits_file_is_rejected() {
        let b = vec![0u8; 2880];
        assert_eq!(FitsHeader::parse(&b), Err(SolveError::NotFits));
    }

    #[test]
    fn a_file_shorter_than_one_block_is_an_error() {
        assert!(matches!(
            FitsHeader::parse(b"SIMPLE  =  T"),
            Err(SolveError::Truncated { .. })
        ));
    }

    #[test]
    fn non_ascii_bytes_do_not_panic() {
        let mut b = header_bytes(&["SIMPLE  =                    T", "BITPIX  =                   16"]);
        b[100] = 0xff;
        b[101] = 0xfe;
        let _ = FitsHeader::parse(&b); // must not panic; result is unimportant
    }

    #[test]
    fn non_finite_values_are_rejected_rather_than_saturated() {
        let b = header_bytes(&[
            "SIMPLE  =                    T",
            "NAXIS1  =                  NAN",
            "NAXIS2  =                1E400",
            "GOOD    =                 3840",
        ]);
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(h.num("NAXIS1"), None, "NaN is corrupt, not a number");
        assert_eq!(h.int("NAXIS2"), None, "an overflowing exponent must not become i64::MAX");
        assert_eq!(h.int("GOOD"), Some(3840));
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn build(cards: &[&str], data: &[u8]) -> Vec<u8> {
        let mut s = String::new();
        for c in cards {
            s.push_str(&format!("{c:<80}"));
        }
        s.push_str(&format!("{:<80}", "END"));
        while !s.len().is_multiple_of(2880) {
            s.push(' ');
        }
        let mut out = s.into_bytes();
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(2880) {
            out.push(0);
        }
        out
    }

    #[test]
    fn decodes_unsigned_16_bit_with_bzero() {
        // What N.I.N.A. writes: BITPIX 16 with BZERO 32768, i.e. unsigned.
        let vals: [u16; 4] = [0, 1000, 40000, 65535];
        let mut data = Vec::new();
        for v in vals {
            let stored = (v as i32 - 32768) as i16;
            data.extend_from_slice(&stored.to_be_bytes());
        }
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "BZERO   =                32768",
                "BSCALE  =                    1",
            ],
            &data,
        );
        let h = FitsHeader::parse(&b).unwrap();
        let img = decode(&b, &h).unwrap();
        assert_eq!((img.nx, img.ny), (2, 2));
        assert_eq!(img.binned, 1);
        for (got, want) in img.px.iter().zip(vals.iter()) {
            assert!((got - *want as f32).abs() < 0.5, "got {got}, want {want}");
        }
    }

    #[test]
    fn decodes_32_bit_float() {
        // What Siril writes.
        let vals: [f32; 4] = [0.0, 0.25, 0.5, 1.0];
        let mut data = Vec::new();
        for v in vals {
            data.extend_from_slice(&v.to_be_bytes());
        }
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                  -32",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
            ],
            &data,
        );
        let h = FitsHeader::parse(&b).unwrap();
        let img = decode(&b, &h).unwrap();
        for (got, want) in img.px.iter().zip(vals.iter()) {
            assert!((got - want).abs() < 1e-6);
        }
    }

    #[test]
    fn a_cfa_frame_is_binned_to_luminance() {
        // 4x4 Bayer mosaic -> 2x2 luminance. Each 2x2 block averages to a
        // known value so the binning is checked, not merely the dimensions.
        let mut data = Vec::new();
        for _ in 0..16 {
            let stored = (1000i32 - 32768) as i16;
            data.extend_from_slice(&stored.to_be_bytes());
        }
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    4",
                "NAXIS2  =                    4",
                "BZERO   =                32768",
                "BAYERPAT= 'RGGB'",
            ],
            &data,
        );
        let h = FitsHeader::parse(&b).unwrap();
        let img = decode(&b, &h).unwrap();
        assert_eq!((img.nx, img.ny), (2, 2), "CFA frames must be 2x2 binned");
        assert_eq!(img.binned, 2, "the binning factor must be reported");
        for p in &img.px {
            assert!((p - 1000.0).abs() < 0.5, "binned value {p} should be the block mean");
        }
    }

    #[test]
    fn truncated_pixel_data_is_an_error_not_a_panic() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                  100",
                "NAXIS2  =                  100",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert!(matches!(decode(&b, &h), Err(SolveError::Truncated { .. })));
    }

    #[test]
    fn an_unsupported_bitpix_is_reported_by_value() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                  -64",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
            ],
            &[0u8; 32],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(decode(&b, &h), Err(SolveError::UnsupportedBitpix(-64)));
    }

    #[test]
    fn an_overflowing_bitpix_is_rejected_not_wrapped_to_a_valid_value() {
        // 4294967312i64 as i32 == 16 -- casting BEFORE the match let a
        // corrupt header decode as if it were ordinary 16-bit data.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =           4294967312",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert!(
            matches!(decode(&b, &h), Err(SolveError::UnsupportedBitpix(_))),
            "a corrupt BITPIX must not decode as if it were 16-bit"
        );
    }

    #[test]
    fn binning_factor_is_2_for_a_cfa_frame_and_1_otherwise() {
        let mono = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    4",
                "NAXIS2  =                    4",
            ],
            &[0u8; 32],
        );
        let h = FitsHeader::parse(&mono).unwrap();
        assert_eq!(binning_factor(&h), 1);

        let cfa = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    4",
                "NAXIS2  =                    4",
                "BAYERPAT= 'RGGB'",
            ],
            &[0u8; 32],
        );
        let h = FitsHeader::parse(&cfa).unwrap();
        assert_eq!(binning_factor(&h), 2, "a BAYERPAT frame must report the binning decode() applies");
    }

    #[test]
    fn implausible_dimensions_are_rejected_before_allocating() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =           1000000000",
                "NAXIS2  =           1000000000",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert!(matches!(decode(&b, &h), Err(SolveError::BadDimensions { .. })));
    }

    #[test]
    fn optics_keywords_give_the_pixel_scale_and_field_height() {
        // The reference rig: 243mm focal length, 2.9um pixels, 2160 rows.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                 3840",
                "NAXIS2  =                 2160",
                "FOCALLEN=                243.0",
                "XPIXSZ  =                  2.9",
                "XBINNING=                    1",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let scale = pixel_scale_arcsec(&h).unwrap();
        assert!((scale - 2.4614).abs() < 0.001, "scale was {scale}");
        let fov = field_height_deg(&h).unwrap();
        assert!((fov - 1.4768).abs() < 0.001, "field height was {fov}");
        let fov_w = field_width_deg(&h).unwrap();
        assert!((fov_w - 2.6255).abs() < 0.001, "field width was {fov_w}");
    }

    #[test]
    fn field_width_deg_is_none_without_optics_keywords() {
        // Mirrors field_height_deg's failure behaviour: no FOCALLEN/XPIXSZ,
        // no pixel scale, no field width -- even though NAXIS1 is present.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                 3840",
                "NAXIS2  =                 2160",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert!(field_width_deg(&h).is_none());
    }

    #[test]
    fn binning_is_accounted_for_in_the_pixel_scale() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                 1920",
                "NAXIS2  =                 1080",
                "FOCALLEN=                243.0",
                "XPIXSZ  =                  2.9",
                "XBINNING=                    2",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let scale = pixel_scale_arcsec(&h).unwrap();
        assert!((scale - 4.9228).abs() < 0.002, "binned scale was {scale}");
    }

    #[test]
    fn the_mount_hint_comes_from_objctra_and_objctdec() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "OBJCTRA = '18 18 48'",
                "OBJCTDEC= '-13 48 26'",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let (ra, dec) = hint_radec(&h).unwrap();
        assert!((ra - 274.7).abs() < 0.01, "ra was {ra}");
        assert!((dec + 13.807).abs() < 0.01, "dec was {dec}");
    }

    #[test]
    fn the_mount_hint_falls_back_to_decimal_ra_dec_for_dwarfiii_frames() {
        // A real DWARFIII frame: RA/DEC in decimal DEGREES, no OBJCTRA/OBJCTDEC
        // at all. ASTAP solved this exact pointing at ra=83.861, dec=-5.397 --
        // the ~2.3' residual against the header value is ordinary
        // commanded-vs-solved pointing error, confirming these are degrees,
        // not hours.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "RA      =             83.82208",
                "DEC     =             -5.39111",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let (ra, dec) = hint_radec(&h).unwrap();
        assert!((ra - 83.82208).abs() < 1e-6, "ra was {ra}");
        assert!((dec + 5.39111).abs() < 1e-6, "dec was {dec}");
    }

    #[test]
    fn a_frame_with_both_pairs_keeps_using_the_sexagesimal_one() {
        // RA/DEC deliberately disagree with OBJCTRA/OBJCTDEC here, so a test
        // that silently preferred the fallback would be caught.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "OBJCTRA = '18 18 48'",
                "OBJCTDEC= '-13 48 26'",
                "RA      =             83.82208",
                "DEC     =             -5.39111",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let (ra, dec) = hint_radec(&h).unwrap();
        assert!((ra - 274.7).abs() < 0.01, "must prefer OBJCTRA/OBJCTDEC, got ra {ra}");
        assert!((dec + 13.807).abs() < 0.01, "must prefer OBJCTRA/OBJCTDEC, got dec {dec}");
    }

    #[test]
    fn decimal_ra_dec_outside_the_valid_range_is_rejected_not_wrapped() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "RA      =                400.0",
                "DEC     =                -95.0",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(hint_radec(&h), None);
    }

    #[test]
    fn decimal_dec_of_exactly_minus_90_is_rejected_as_an_unset_sentinel() {
        // A real UnknownOBJECT capture carries DEC = -90. with no genuine
        // pointing intent behind it. Accepting it silently would seed a
        // catalogue search around the wrong point instead of correctly
        // reporting no usable hint.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "RA      =                120.0",
                "DEC     =               -90.00",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(hint_radec(&h), None, "DEC=-90 is a known sentinel, not a real pole pointing");
    }

    #[test]
    fn a_non_finite_ra_or_dec_is_treated_as_absent() {
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "RA      =                  NAN",
                "DEC     =                -5.39",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        assert_eq!(hint_radec(&h), None, "a NaN RA must not be treated as a usable hint");
    }

    #[test]
    fn date_obs_is_read_as_utc_and_converted_to_a_decimal_year() {
        // DATE-OBS carries no timezone and N.I.N.A. writes it in UTC. Reading it
        // as local time is a 12-hour error at this site and has bitten this
        // project twice.
        let b = build(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                   16",
                "NAXIS   =                    2",
                "NAXIS1  =                    2",
                "NAXIS2  =                    2",
                "DATE-OBS= '2026-07-29T10:47:02.6926650'",
            ],
            &[0u8; 8],
        );
        let h = FitsHeader::parse(&b).unwrap();
        let y = epoch_years(&h).unwrap();
        assert!((y - 2026.57).abs() < 0.02, "epoch was {y}");
    }
}
