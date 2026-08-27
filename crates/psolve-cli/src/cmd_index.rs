use crate::flag;
use psolve_index::builder::Builder;
use psolve_index::gaia::{read_ecsv, ColumnNames, RowFilter};
use psolve_index::record::StarRecord;
use psolve_index::sha256::hex;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// Plain `.csv` only, plus a count of compressed files we had to skip.
///
/// psolve has no gzip decoder -- the dependency budget is memmap2 + rayon --
/// so a `.csv.gz` here would be parsed as binary, fail the header lookup, and
/// be skipped with a warning. That silently yields a SHORT index, which is
/// the worst possible failure: it looks like a successful build. Reject
/// compressed input loudly instead.
fn csv_files(dir: &Path) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let mut out = Vec::new();
    let mut compressed = 0usize;
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if !p.is_file() {
            continue;
        }
        let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if n.ends_with(".gz") || n.ends_with(".bz2") || n.ends_with(".zst") {
            compressed += 1;
        } else if n.ends_with(".csv") {
            out.push(p);
        }
    }
    out.sort();
    Ok((out, compressed))
}

/// What the local catalogue mirror actually contains, from `mirror.json`
/// written by fetch-gaia.sh. Hand-parsed: the dependency budget has no JSON
/// crate, and this reads a handful of fields from a file we wrote ourselves.
enum Mirror {
    /// No mirror.json: a bring-your-own directory, validation genuinely does not apply.
    Absent,
    /// Present but unparseable (truncated write, hand-edited, corrupt). NOT the
    /// same as absent: silently skipping the guard here would reopen exactly
    /// the silently-short-index hole the guard exists to close.
    Unreadable,
    Present {
        max_mag: f32,
        min_dec: f64,
        max_dec: f64,
        /// Number of source files the fetch was meant to cover, if recorded.
        files: Option<u64>,
        /// Whether the fetch that wrote this manifest ran to completion.
        /// Absent (an older fetch-gaia.sh, or a hand-built manifest) is
        /// treated as `true`: the `files` count below is the guard that
        /// actually catches an incomplete fetch either way.
        complete: bool,
    },
}

fn read_mirror(dir: &Path) -> Mirror {
    let path = dir.join("mirror.json");
    if !path.exists() {
        return Mirror::Absent;
    }
    let Ok(txt) = std::fs::read_to_string(&path) else {
        return Mirror::Unreadable;
    };
    let num = |key: &str| -> Option<f64> {
        let at = txt.find(&format!("\"{key}\""))?;
        let after_colon = txt[at..].find(':')? + at + 1;
        let val: String = txt[after_colon..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            .collect();
        val.parse().ok()
    };
    let boolean = |key: &str| -> Option<bool> {
        let at = txt.find(&format!("\"{key}\""))?;
        let after_colon = txt[at..].find(':')? + at + 1;
        let rest = txt[after_colon..].trim_start();
        if rest.starts_with("true") {
            Some(true)
        } else if rest.starts_with("false") {
            Some(false)
        } else {
            None
        }
    };
    let files = num("files").filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64);
    let complete = boolean("complete").unwrap_or(true);
    match (num("max_mag"), num("min_dec"), num("max_dec")) {
        (Some(m), Some(lo), Some(hi)) if m.is_finite() && lo.is_finite() && hi.is_finite() => {
            Mirror::Present { max_mag: m as f32, min_dec: lo, max_dec: hi, files, complete }
        }
        _ => Mirror::Unreadable,
    }
}

pub fn build(args: &[&str]) -> ExitCode {
    let Some(input) = flag(args, "--input") else {
        eprintln!("psolve index build: --input <DIR> is required");
        return ExitCode::from(2);
    };
    let Some(out) = flag(args, "--out") else {
        eprintln!("psolve index build: --out <FILE> is required");
        return ExitCode::from(2);
    };
    let max_mag: f32 = match flag(args, "--max-mag").unwrap_or("14").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --max-mag must be a number");
            return ExitCode::from(2);
        }
    };
    // `f32::from_str` accepts "NaN"/"inf"/"-inf" as valid floats, and
    // RowFilter::validate() below only bounds-checks the declination range --
    // it has no opinion on magnitude, since f32::INFINITY is its own legitimate
    // "no limit" default. Left unchecked here, a non-finite --max-mag would
    // make every row's `mag <= max_mag` comparison false and silently build a
    // zero-record index instead of failing loudly.
    if !max_mag.is_finite() {
        eprintln!("psolve index build: --max-mag must be a finite number");
        return ExitCode::from(2);
    }
    let nside: u32 = match flag(args, "--nside").unwrap_or("64").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --nside must be an integer");
            return ExitCode::from(2);
        }
    };
    let epoch: f64 = match flag(args, "--epoch").unwrap_or("2016.0").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --epoch must be a decimal year");
            return ExitCode::from(2);
        }
    };
    // Same shape as the --max-mag guard above: "NaN"/"inf" parse fine as f64
    // but would flow straight into the JSON result as a bare, invalid token
    // (`"epoch":NaN`) while still exiting 0.
    if !epoch.is_finite() {
        eprintln!("psolve index build: --epoch must be a finite decimal year");
        return ExitCode::from(2);
    }
    // Declination limits: a fixed site never sees the whole sky.
    let parse_dec = |name: &str, default: &str| -> Result<f64, ()> {
        flag(args, name).unwrap_or(default).parse::<f64>().map_err(|_| ())
    };
    let (min_dec, max_dec) = match (parse_dec("--min-dec", "-90"), parse_dec("--max-dec", "90")) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            eprintln!("psolve index build: --min-dec/--max-dec must be degrees");
            return ExitCode::from(2);
        }
    };
    let filter = RowFilter { max_mag, min_dec, max_dec };
    if let Err(e) = filter.validate() {
        eprintln!("psolve index build: {e}");
        return ExitCode::from(2);
    }
    let names = match flag(args, "--columns") {
        None => ColumnNames::default(),
        Some(spec) => match ColumnNames::with_overrides(spec) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("psolve index build: {e}");
                return ExitCode::from(2);
            }
        },
    };

    // "gaia-dr3-..." is only true when the catalogue actually is Gaia DR3's
    // own column layout. `--columns` means the caller pointed this at some
    // other catalogue (Tycho-2, Vizier, ...), so stamping a Gaia-branded name
    // on it by default would be actively misleading -- use a neutral name
    // instead unless `--name` overrides it explicitly.
    let default_name = if flag(args, "--columns").is_some() {
        format!("catalog-g{}-nside{}", max_mag as i32, nside)
    } else {
        format!("gaia-dr3-g{}-nside{}", max_mag as i32, nside)
    };
    let name = flag(args, "--name").unwrap_or(&default_name);
    // The name is interpolated unescaped into the JSON result below and stored
    // in a fixed 32-byte header field. Reject anything that would break
    // either, rather than escaping it: it's a short identifier, not free text.
    if name.is_empty()
        || name.len() > 32
        || !name.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
    {
        eprintln!(
            "psolve index build: --name must be 1-32 printable ASCII characters \
             with no quotes or backslashes"
        );
        return ExitCode::from(2);
    }
    // A malformed --jobs must not be silently ignored in favour of the
    // default thread count: that is the same "launder bad input into a
    // plausible default" shape as the --max-mag gap above, just for a flag
    // whose failure mode is easy to miss (the build still succeeds, just
    // not with the parallelism the caller asked for).
    if let Some(v) = flag(args, "--jobs") {
        match v.parse::<usize>() {
            // rayon treats num_threads(0) as "use the default", which would
            // launder a bogus --jobs value into the default thread count
            // instead of rejecting it -- the same "bad input silently
            // becomes a plausible default" shape as everything else guarded
            // above.
            Ok(0) => {
                eprintln!("psolve index build: --jobs must be at least 1");
                return ExitCode::from(2);
            }
            Ok(j) => {
                let _ = rayon::ThreadPoolBuilder::new().num_threads(j).build_global();
            }
            Err(_) => {
                eprintln!("psolve index build: --jobs must be a non-negative integer");
                return ExitCode::from(2);
            }
        }
    }
    // Opt back into a non-fatal result for two conditions that otherwise exit
    // 3 below: per-file read failures (a malformed row truncates that file's
    // contribution) and a mirror manifest that is not provably complete. Both
    // still report exactly what happened -- this flag silences the failure
    // exit code, not the visibility of what was lost.
    let allow_partial = args.contains(&"--allow-partial");

    let dir = Path::new(input);
    let (files, compressed) = match csv_files(dir) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("psolve index build: cannot read {input}: {e}");
            return ExitCode::from(2);
        }
    };
    // Compressed input is refused outright, not merely warned about: psolve
    // has no gzip decoder, so a `.csv.gz` here would fail column lookup and
    // be skipped, which silently yields a SHORT index -- the worst possible
    // failure, because it looks like a successful build. See the module-level
    // comment on `csv_files` above.
    if compressed > 0 {
        eprintln!(
            "psolve index build: {compressed} compressed file(s) in {input} \
             -- psolve has no gzip decoder. Decompress them first."
        );
        return ExitCode::from(2);
    }
    if files.is_empty() {
        eprintln!("psolve index build: no .csv files in {input}");
        return ExitCode::from(2);
    }

    // Never build deeper or wider than the mirror actually holds: doing so
    // produces an index that is silently short, which looks exactly like a
    // successful build. A present-but-unparseable mirror.json must refuse
    // too -- "absent" and "unreadable" are not the same outcome.
    match read_mirror(dir) {
        Mirror::Absent => {}
        Mirror::Unreadable => {
            eprintln!(
                "psolve index build: {input}/mirror.json exists but could not be parsed. \
                 Refusing to build without a validated mirror guard; fix or remove the file."
            );
            return ExitCode::from(2);
        }
        Mirror::Present { max_mag: m_mag, min_dec: m_min, max_dec: m_max, files: m_files, complete } =>
        {
            // An interrupted fetch leaves shards/ populated but the manifest
            // never gets its final rewrite -- unless fetch-gaia.sh writes an
            // upfront "complete":false marker before the long-running fetch,
            // in which case an interruption leaves exactly that marker
            // behind. Building from a mirror known to be incomplete produces
            // an index that is silently short in exactly the way this whole
            // guard exists to prevent.
            if !complete && !allow_partial {
                eprintln!(
                    "psolve index build: {input}/mirror.json records an incomplete fetch \
                     (\"complete\":false). The shard directory may be missing files; \
                     re-run fetch-gaia.sh to finish it, or pass --allow-partial to build \
                     anyway."
                );
                return ExitCode::from(3);
            }
            if let Some(expected) = m_files {
                let actual = files.len() as u64;
                if actual < expected && !allow_partial {
                    eprintln!(
                        "psolve index build: {input} has {actual} .csv file(s) but \
                         mirror.json recorded {expected}. The fetch looks incomplete; \
                         re-run fetch-gaia.sh, or pass --allow-partial to build anyway."
                    );
                    return ExitCode::from(3);
                }
            }
            if max_mag > m_mag + 1e-6 {
                eprintln!(
                    "psolve index build: --max-mag {max_mag} is deeper than this mirror, which \
                     was fetched to {m_mag}. Re-fetch deeper or lower --max-mag; building anyway \
                     would produce a silently shallow index."
                );
                return ExitCode::from(2);
            }
            if min_dec < m_min - 1e-9 || max_dec > m_max + 1e-9 {
                eprintln!(
                    "psolve index build: declination range {min_dec}..{max_dec} is wider than \
                     this mirror's {m_min}..{m_max}. Re-fetch wider or narrow the range."
                );
                return ExitCode::from(2);
            }
        }
    }

    let mut builder = match Builder::new(nside, max_mag, epoch, name) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("psolve index build: {e}");
            return ExitCode::from(2);
        }
    };

    let t0 = Instant::now();
    eprintln!(
        "reading {} file(s) from {input} (mag<={max_mag}, dec {min_dec}..{max_dec})",
        files.len()
    );

    // Files are read in parallel; each yields its own row vector, then rows are
    // pushed into the single builder. The sort happens once, in finish().
    //
    // `read_ecsv` aborts at the first malformed row in a file, so a failed
    // file still contributes whatever it parsed before the bad row (kept,
    // not discarded) but is also short by everything after it. That must not
    // be a silent truncation: it is tracked as a failed file below and turns
    // into a non-zero exit unless the caller opted into --allow-partial. A
    // file that cannot even be opened gets the same treatment.
    let per_file: Vec<(Vec<psolve_index::gaia::GaiaRow>, bool)> = files
        .par_iter()
        .map(|p| {
            let mut rows = Vec::new();
            let mut failed = false;
            match File::open(p) {
                Ok(f) => {
                    if let Err(e) = read_ecsv(BufReader::new(f), &names, &filter, |r| rows.push(r))
                    {
                        eprintln!("  warn: {}: {e}", p.display());
                        failed = true;
                    }
                }
                Err(e) => {
                    eprintln!("  warn: {}: {e}", p.display());
                    failed = true;
                }
            }
            (rows, failed)
        })
        .collect();

    let mut files_failed: u64 = 0;
    for (rows, failed) in per_file {
        if failed {
            files_failed += 1;
        }
        for r in rows {
            builder.push(r.ra, r.dec, r.mag, r.pmra, r.pmdec);
        }
    }

    let mut f = match File::create(out) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("psolve index build: cannot create {out}: {e}");
            return ExitCode::from(2);
        }
    };
    let stats = match builder.finish(&mut f) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("psolve index build: {e}");
            return ExitCode::from(3);
        }
    };
    drop(f);

    let digest = match psolve_index::reader::Index::open(Path::new(out)) {
        Ok(i) => hex(&i.header().records_sha256),
        Err(e) => {
            eprintln!("psolve index build: wrote an index that will not open: {e}");
            return ExitCode::from(3);
        }
    };

    println!(
        "{{\"n_records\":{},\"clamped\":{},\"skipped\":{},\"files_failed\":{},\"nside\":{},\
\"max_mag\":{},\"min_dec\":{},\"max_dec\":{},\"epoch\":{},\"name\":\"{}\",\"sha256\":\"{}\",\
\"seconds\":{:.1}}}",
        stats.written,
        stats.clamped,
        stats.skipped,
        files_failed,
        nside,
        max_mag,
        min_dec,
        max_dec,
        epoch,
        name,
        digest,
        t0.elapsed().as_secs_f64()
    );
    // A file that failed mid-parse means the index is short by an unknown
    // amount from that file -- exactly the silently-short-index failure this
    // whole guard exists to catch, so it must not exit 0 by default. The
    // JSON above is still printed either way: --allow-partial silences the
    // exit code, not the visibility of what was lost.
    if files_failed > 0 && !allow_partial {
        return ExitCode::from(3);
    }
    ExitCode::SUCCESS
}

/// Escapes a string for embedding in a JSON string literal.
///
/// `h.name_str()` comes straight off disk (a 32-byte header field with no
/// content restriction on read, unlike `build`'s `--name` guard on write) --
/// a corrupt or hand-edited index can carry a `"` or `\` in it, which would
/// otherwise break out of the surrounding quotes and hand the caller
/// malformed JSON on stdout while still exiting 0. Control characters are
/// escaped too, since name_str() only guarantees valid UTF-8, not printable.
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Renders a header-sourced f64 as a JSON number, refusing to hand out a
/// bare `NaN`/`inf` token: `h.epoch` is read off disk (see `json_escape`
/// above for why that's untrusted), and `f64`'s `Display` prints exactly
/// those non-finite tokens, which are not valid JSON.
pub(crate) fn json_number(v: f64) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "null".to_string()
    }
}

pub fn info(args: &[&str]) -> ExitCode {
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("psolve index info: <FILE> is required");
        return ExitCode::from(2);
    };
    let verify = args.contains(&"--verify");

    let idx = match psolve_index::reader::Index::open(Path::new(path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve index info: {e}");
            return ExitCode::from(3);
        }
    };
    let h = idx.header();
    let name = json_escape(h.name_str());

    let npix = psolve_index::healpix::npix(h.nside);
    let mut occupied = 0u64;
    let mut max_cell = 0usize;
    for c in 0..npix {
        let l = idx.cell_len(c);
        if l > 0 {
            occupied += 1;
        }
        if l > max_cell {
            max_cell = l;
        }
    }

    // On a failed --verify, exit 3 for "index problem" and print nothing to
    // stdout: a partial JSON object here would be two contradictory signals
    // at once (exit 3 says "problem", partial-but-valid JSON says "here is
    // a result"), and callers that only check exit code before parsing
    // stdout would still misread a malformed object as success.
    if verify {
        if let Err(e) = idx.verify_digest() {
            eprintln!("psolve index info: {e}");
            return ExitCode::from(3);
        }
    }

    // `verified` says whether --verify ran at all; `digest_ok` says what it
    // found. Without --verify the digest was never checked, so `digest_ok`
    // must not print `false` -- that reads as "checked and failed" when
    // really nothing was checked. It is `null` (only reachable here, since a
    // failed verify above already returned) whenever `verified` is false.
    let digest_ok = if verify { "true".to_string() } else { "null".to_string() };
    println!(
        "{{\"name\":\"{}\",\"version\":{},\"nside\":{},\"npix\":{},\"epoch\":{},\
\"n_records\":{},\"max_mag\":{},\"occupied_cells\":{},\"max_cell_records\":{},\
\"mean_per_occupied_cell\":{:.1},\"sha256\":\"{}\",\"verified\":{},\"digest_ok\":{}}}",
        name,
        h.version,
        h.nside,
        npix,
        json_number(h.epoch),
        h.n_records,
        json_number(h.mag_limit as f64),
        occupied,
        max_cell,
        if occupied > 0 { h.n_records as f64 / occupied as f64 } else { 0.0 },
        hex(&h.records_sha256),
        verify,
        digest_ok
    );
    ExitCode::SUCCESS
}

/// Flags that consume the following token, for `index query`'s positional
/// scan below. Every flag that takes a value must be listed here or its
/// value can be mistaken for the positional `<INDEX>` argument -- the same
/// defect `cmd_solve.rs`'s `VALUED_FLAGS` guards against, and it has shipped
/// twice in this repo (see that module's own doc comment).
const QUERY_VALUED_FLAGS: &[&str] = &["--ra", "--dec", "--radius", "--max-mag", "--format"];

/// The first token that is neither a flag nor a flag's value -- mirrors
/// `cmd_solve::positional`'s reasoning exactly: a naive
/// `find(|a| !a.starts_with("--"))` would bind a valued flag's own value
/// (e.g. the `2.0` in `--radius 2.0`) as the index path instead of skipping
/// past it.
fn query_positional<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if QUERY_VALUED_FLAGS.contains(&a) {
            i += 2;
        } else if a.starts_with("--") {
            i += 1;
        } else {
            return Some(a);
        }
    }
    None
}

/// Look up an *optional* value-consuming flag, distinguishing "absent
/// entirely" (`Ok(None)`, the caller should apply its own default) from
/// "present but with nothing after it" (`Err`, always a usage error).
///
/// `crate::flag` alone cannot make that distinction -- it returns `None` for
/// both -- which is exactly right for a *required* flag (either way it is
/// "usage error: missing") but wrong here: an optional flag with a
/// meaningful default must not silently fall back to that default just
/// because the caller typo'd or a shell variable expanded to nothing. A
/// build script doing `--max-mag $CAP` with `$CAP` unset expands to a bare
/// trailing `--max-mag`, and this repo's own brief for this subcommand
/// explicitly rules out that expanding silently into "no cap at all" over a
/// 9,495-frame backfill.
fn optional_flag<'a>(args: &[&'a str], name: &'static str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|a| *a == name) {
        None => Ok(None),
        Some(i) => match args.get(i + 1) {
            Some(v) => Ok(Some(v)),
            None => Err(format!("{name} requires a value, got none")),
        },
    }
}

enum QueryFormat {
    Csv,
    Ndjson,
}

/// Write one star as a CSV data row -- no header, no trailing state. Column
/// order and precision mirror the reference dumps this subcommand exists to
/// reproduce: ra/dec to the microarcsecond-ish 9 decimal places, magnitude to
/// the millimag, proper motion to the milliarcsec/yr (the record's own
/// stored precision, since `pmra_mas`/`pmdec_mas` are integral internally).
fn write_csv_row(out: &mut impl Write, r: &StarRecord) -> std::io::Result<()> {
    writeln!(
        out,
        "{:.9},{:.9},{:.4},{:.3},{:.3}",
        r.ra_deg(),
        r.dec_deg(),
        r.mag(),
        r.pmra_mas_yr(),
        r.pmdec_mas_yr()
    )
}

/// Write one star as an NDJSON line: a self-contained JSON object per row, no
/// enclosing array -- so a consumer can start processing before the query
/// finishes and a truncated stream is still valid up to its last complete
/// line.
fn write_ndjson_row(out: &mut impl Write, r: &StarRecord) -> std::io::Result<()> {
    writeln!(
        out,
        "{{\"ra_deg\":{:.9},\"dec_deg\":{:.9},\"phot_g_mean_mag\":{:.4},\
\"pmra_mas_yr\":{:.3},\"pmdec_mas_yr\":{:.3}}}",
        r.ra_deg(),
        r.dec_deg(),
        r.mag(),
        r.pmra_mas_yr(),
        r.pmdec_mas_yr()
    )
}

/// `psolve index query <INDEX> --ra <deg> --dec <deg> --radius <deg>
/// [--max-mag <m>] [--format csv|ndjson]` -- every catalogue star in a disc,
/// to a magnitude cap, with no brightest-N truncation. Built for an absolute
/// transparency (limiting-magnitude) measurement, which needs the true
/// per-frame star count to a depth, not the brightest handful:
/// `Index::stars_in_disc` (this subcommand's one call into psolve-index) has
/// the full reasoning for why that is a different query from
/// `brightest_in_disc`, not just the same one with a bigger limit.
///
/// Stdout carries data only. `stars_in_disc` still materialises the full
/// result as a `Vec<StarRecord>` before any row is written -- at 16 bytes a
/// record that's a few MB even for the largest discs this subcommand sees,
/// not a concern here -- but the WRITE path is what streams: each row is
/// `writeln!`'d straight to a `BufWriter` as the vector is iterated, rather
/// than formatted into one giant `String` first and printed in one shot.
/// Everything else (errors, the summary line) goes to stderr.
pub fn query(args: &[&str]) -> ExitCode {
    let Some(path) = query_positional(args) else {
        eprintln!("psolve index query: <INDEX> is required");
        return ExitCode::from(2);
    };

    let ra_deg = match flag(args, "--ra") {
        None => {
            eprintln!("psolve index query: --ra <deg> is required");
            return ExitCode::from(2);
        }
        Some(v) => match v.parse::<f64>() {
            Ok(x) if x.is_finite() && (0.0..=360.0).contains(&x) => x,
            _ => {
                eprintln!("psolve index query: --ra must be a finite number in 0..360");
                return ExitCode::from(2);
            }
        },
    };
    let dec_deg = match flag(args, "--dec") {
        None => {
            eprintln!("psolve index query: --dec <deg> is required");
            return ExitCode::from(2);
        }
        Some(v) => match v.parse::<f64>() {
            Ok(x) if x.is_finite() && (-90.0..=90.0).contains(&x) => x,
            _ => {
                eprintln!("psolve index query: --dec must be a finite number in -90..90");
                return ExitCode::from(2);
            }
        },
    };
    let radius_deg = match flag(args, "--radius") {
        None => {
            eprintln!("psolve index query: --radius <deg> is required");
            return ExitCode::from(2);
        }
        Some(v) => match v.parse::<f64>() {
            Ok(x) if x.is_finite() && x > 0.0 => x,
            _ => {
                eprintln!(
                    "psolve index query: --radius must be a positive finite number of degrees"
                );
                return ExitCode::from(2);
            }
        },
    };
    // Deferred to the index's own mag_limit only once the index is open
    // (below) -- absent here, this just records whether an override was
    // given at all, and validates it eagerly so a typo is a usage error
    // rather than a silent fall-through to the default. `optional_flag`
    // (not plain `flag`) so a trailing, valueless `--max-mag` is a usage
    // error rather than silently taking the absent-flag branch below.
    let max_mag_override = match optional_flag(args, "--max-mag") {
        Err(e) => {
            eprintln!("psolve index query: {e}");
            return ExitCode::from(2);
        }
        Ok(None) => None,
        Ok(Some(v)) => match v.parse::<f32>() {
            Ok(x) if x.is_finite() => Some(x),
            _ => {
                eprintln!("psolve index query: --max-mag must be a finite number");
                return ExitCode::from(2);
            }
        },
    };
    let format = match optional_flag(args, "--format") {
        Err(e) => {
            eprintln!("psolve index query: {e}");
            return ExitCode::from(2);
        }
        Ok(None) | Ok(Some("csv")) => QueryFormat::Csv,
        Ok(Some("ndjson")) => QueryFormat::Ndjson,
        Ok(Some(other)) => {
            eprintln!("psolve index query: --format must be csv or ndjson, got {other:?}");
            return ExitCode::from(2);
        }
    };

    let idx = match psolve_index::reader::Index::open(Path::new(path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve index query: {e}");
            return ExitCode::from(3);
        }
    };
    // Inclusive cut, and defaults to the index's own build-time depth: a
    // caller asking for "everything" without naming a cap should get
    // everything the index actually holds, not an arbitrary narrower slice.
    let max_mag = max_mag_override.unwrap_or(idx.header().mag_limit);

    let t0 = Instant::now();
    let stars = idx.stars_in_disc(ra_deg, dec_deg, radius_deg, max_mag);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let write_result: std::io::Result<()> = (|| {
        if matches!(format, QueryFormat::Csv) {
            writeln!(out, "ra_deg,dec_deg,phot_g_mean_mag,pmra_mas_yr,pmdec_mas_yr")?;
        }
        for r in &stars {
            match format {
                QueryFormat::Csv => write_csv_row(&mut out, r)?,
                QueryFormat::Ndjson => write_ndjson_row(&mut out, r)?,
            }
        }
        out.flush()
    })();
    if let Err(e) = write_result {
        eprintln!("psolve index query: error writing output: {e}");
        return ExitCode::from(3);
    }

    eprintln!(
        "psolve index query: {} star(s) within {radius_deg} deg of {ra_deg:.4},{dec_deg:.4} \
(mag<={max_mag}) in {:.3}s",
        stars.len(),
        t0.elapsed().as_secs_f64()
    );

    ExitCode::SUCCESS
}

#[cfg(test)]
mod query_tests {
    use super::{optional_flag, query_positional, QUERY_VALUED_FLAGS};

    /// Every flag that takes a value must be in QUERY_VALUED_FLAGS, or the
    /// positional scan binds the flag's value as the index path instead of
    /// the real one. This defect has shipped twice already in this repo
    /// (`cmd_solve.rs`'s own test names the M2/T13 occurrence) -- one guard
    /// test per positional scanner keeps a third occurrence from being silent.
    #[test]
    fn every_valued_flag_is_registered_for_the_positional_scan() {
        for f in ["--ra", "--dec", "--radius", "--max-mag", "--format"] {
            assert!(
                QUERY_VALUED_FLAGS.contains(&f),
                "{f} takes a value but is not registered; the positional scan \
                 will bind its value as the index path"
            );
        }
    }

    #[test]
    fn positional_skips_every_valued_flag_and_its_value() {
        let args = [
            "--ra", "10.0", "--dec", "-20.0", "--radius", "2.0", "--max-mag", "14", "--format",
            "csv", "index.psidx",
        ];
        assert_eq!(query_positional(&args), Some("index.psidx"));
    }

    #[test]
    fn positional_finds_the_index_path_before_the_flags_too() {
        let args = ["index.psidx", "--ra", "10.0", "--dec", "-20.0", "--radius", "2.0"];
        assert_eq!(query_positional(&args), Some("index.psidx"));
    }

    #[test]
    fn optional_flag_is_ok_none_when_the_flag_is_absent_entirely() {
        let args = ["--ra", "10.0"];
        assert_eq!(optional_flag(&args, "--max-mag"), Ok(None));
        assert_eq!(optional_flag(&args, "--format"), Ok(None));
    }

    #[test]
    fn optional_flag_is_ok_some_when_a_value_follows() {
        let args = ["--max-mag", "14.0"];
        assert_eq!(optional_flag(&args, "--max-mag"), Ok(Some("14.0")));
    }

    /// The bug this repo's own coordinator found: a flag present with
    /// nothing after it must not be indistinguishable from the flag being
    /// absent -- `optional_flag` must return `Err`, not `Ok(None)`, so the
    /// caller cannot silently fall back to its default. Covers both the
    /// trailing-in-argv case (`--max-mag` is the last token) and the
    /// immediately-followed-by-another-flag case (`--max-mag --format` --
    /// `--format` is `--max-mag`'s next token positionally, not a value for
    /// it, but `optional_flag` itself does not know that; the eager
    /// `.parse()` at the call site is what rejects it, so this only checks
    /// the pure trailing case here).
    #[test]
    fn optional_flag_errors_when_present_with_no_value_following() {
        let args = ["--ra", "10.0", "--max-mag"];
        assert!(optional_flag(&args, "--max-mag").is_err());

        let args2 = ["--ra", "10.0", "--format"];
        assert!(optional_flag(&args2, "--format").is_err());
    }
}
