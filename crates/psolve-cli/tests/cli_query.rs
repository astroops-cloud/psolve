use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-cli-query-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SAMPLE: &str = include_str!("../../psolve-index/tests/fixtures/gaia_sample.csv");

/// Builds a small index from the shared Gaia sample fixture (3 rows survive
/// its one malformed/null row -- see `gaia.rs`'s row filter) and returns its
/// path alongside the temp dir that owns it.
fn built_index(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = tmpdir(tag);
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("i.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64", "--name", "query-test"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    (d, out)
}

#[test]
fn query_default_csv_has_the_exact_header_and_every_row() {
    let (_d, idx) = built_index("csv-default");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("ra_deg,dec_deg,phot_g_mean_mag,pmra_mas_yr,pmdec_mas_yr"),
        "header row must match exactly"
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), 3, "all 3 built stars are within 1 deg of (45,0): {stdout}");
    for row in &rows {
        assert_eq!(row.split(',').count(), 5, "row {row:?} must have 5 fields");
    }
}

#[test]
fn query_ndjson_emits_one_json_object_per_line() {
    let (_d, idx) = built_index("ndjson");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--format", "ndjson"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert!(line.starts_with('{') && line.ends_with('}'), "not a JSON object: {line}");
        for key in ["\"ra_deg\"", "\"dec_deg\"", "\"phot_g_mean_mag\"", "\"pmra_mas_yr\"", "\"pmdec_mas_yr\""]
        {
            assert!(line.contains(key), "missing {key} in {line}");
        }
    }
}

#[test]
fn stdout_is_data_only_summary_goes_to_stderr() {
    let (_d, idx) = built_index("stdout-data-only");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let stdout = String::from_utf8_lossy(&o.stdout);
    for line in stdout.lines() {
        assert!(
            line == "ra_deg,dec_deg,phot_g_mean_mag,pmra_mas_yr,pmdec_mas_yr"
                || line.split(',').count() == 5,
            "unexpected non-data line on stdout: {line}"
        );
    }
    assert!(!o.stderr.is_empty(), "the summary belongs on stderr");
}

#[test]
fn max_mag_defaults_to_the_indexs_own_mag_limit() {
    // Built at --max-mag 20 above, so a query with no --max-mag override must
    // include the faintest sample row (mag 17.641426) rather than silently
    // capping at some other default.
    let (_d, idx) = built_index("default-mag-limit");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("17.6410"), "faintest sample star must survive the default cap: {stdout}");
}

#[test]
fn max_mag_override_excludes_fainter_stars() {
    let (_d, idx) = built_index("mag-override");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--max-mag", "15.0"])
        .output()
        .unwrap();
    assert!(o.status.success());
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(!stdout.contains("17.6410"), "mag 17.6 star must be excluded by --max-mag 15: {stdout}");
    assert!(stdout.contains("14.1280"), "mag 14.1 star must survive --max-mag 15: {stdout}");
}

#[test]
fn missing_index_path_is_a_usage_error() {
    let o = Command::new(bin())
        .args(["index", "query", "--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn missing_required_flags_are_usage_errors() {
    let (_d, idx) = built_index("missing-flags");
    for args in [
        vec!["--dec".to_string(), "0.0".to_string(), "--radius".to_string(), "1.0".to_string()],
        vec!["--ra".to_string(), "45.0".to_string(), "--radius".to_string(), "1.0".to_string()],
        vec!["--ra".to_string(), "45.0".to_string(), "--dec".to_string(), "0.0".to_string()],
    ] {
        let o = Command::new(bin())
            .args(["index", "query"])
            .arg(&idx)
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "args {args:?} must be a usage error");
    }
}

#[test]
fn out_of_range_ra_dec_and_non_positive_radius_are_usage_errors() {
    let (_d, idx) = built_index("bad-ranges");
    for args in [
        ["--ra", "400.0", "--dec", "0.0", "--radius", "1.0"],
        ["--ra", "45.0", "--dec", "-95.0", "--radius", "1.0"],
        ["--ra", "45.0", "--dec", "0.0", "--radius", "0.0"],
        ["--ra", "45.0", "--dec", "0.0", "--radius", "-1.0"],
        ["--ra", "nan", "--dec", "0.0", "--radius", "1.0"],
        ["--ra", "45.0", "--dec", "0.0", "--radius", "inf"],
    ] {
        let o = Command::new(bin())
            .args(["index", "query"])
            .arg(&idx)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "args {args:?} must be a usage error");
    }
}

#[test]
fn non_finite_max_mag_is_a_usage_error() {
    let (_d, idx) = built_index("bad-max-mag");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--max-mag", "nan"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn unknown_format_is_a_usage_error() {
    let (_d, idx) = built_index("bad-format");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--format", "xml"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

/// A value-consuming flag present with nothing after it must be a usage
/// error, not a silent fall-back to the default -- verified against the
/// live binary, not just the unit-level `optional_flag` helper. A build
/// script's `--max-mag $CAP` expanding to a bare trailing `--max-mag` when
/// `$CAP` is unset is exactly this shape, and it must fail loudly rather
/// than silently querying at the index's full depth.
#[test]
fn trailing_max_mag_with_no_value_is_a_usage_error_not_a_silent_default() {
    let (_d, idx) = built_index("trailing-max-mag");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--max-mag"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(2),
        "a valueless trailing --max-mag must be a usage error, not exit 0: stdout {}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(o.stdout.is_empty(), "a usage error must not emit data rows");
}

/// Same defect, `--format` half: a bare trailing `--format` must not
/// silently default to csv.
#[test]
fn trailing_format_with_no_value_is_a_usage_error_not_a_silent_default() {
    let (_d, idx) = built_index("trailing-format");
    let o = Command::new(bin())
        .args(["index", "query"])
        .arg(&idx)
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0", "--format"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(2),
        "a valueless trailing --format must be a usage error, not exit 0: stdout {}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(o.stdout.is_empty(), "a usage error must not emit data rows");
}

#[test]
fn a_missing_index_file_exits_3() {
    let o = Command::new(bin())
        .args(["index", "query", "/nonexistent/none.psidx"])
        .args(["--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

/// A flag's value must never be mistaken for the positional `<INDEX>` path --
/// the defect this repo has shipped twice already for other positional
/// scanners. Placing the index path AFTER the flags exercises exactly that.
#[test]
fn the_index_path_is_found_regardless_of_where_it_falls_among_the_flags() {
    let (_d, idx) = built_index("flag-order");
    let o = Command::new(bin())
        .args(["index", "query", "--ra", "45.0", "--dec", "0.0", "--radius", "1.0"])
        .arg(&idx)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout).lines().count(), 4, "header + 3 rows");
}
