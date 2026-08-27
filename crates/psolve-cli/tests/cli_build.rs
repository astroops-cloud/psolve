use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("psolve{}", std::env::consts::EXE_SUFFIX))
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SAMPLE: &str = include_str!("../../psolve-index/tests/fixtures/gaia_sample.csv");

#[test]
fn builds_an_index_from_a_directory_of_csv() {
    let d = tmpdir("build");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("GaiaSource_000000-000001.csv"), SAMPLE).unwrap();
    let out = d.join("out.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64", "--name", "test-build"])
        .output()
        .unwrap();

    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out.exists(), "index file was not created");

    let idx = psolve_index::reader::Index::open(&out).unwrap();
    assert_eq!(idx.header().n_records, 3);
    assert_eq!(idx.header().name_str(), "test-build");
    assert_eq!(idx.header().nside, 64);
    idx.verify_digest().unwrap();
}

#[test]
fn build_emits_json_on_stdout_and_progress_on_stderr() {
    let d = tmpdir("streams");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("out.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.trim_start().starts_with('{'), "stdout must be JSON, got: {stdout}");
    assert!(stdout.contains("\"n_records\""));
    // Crude but effective: a bare NaN/inf token is not valid JSON, and would
    // have caught the --epoch NaN defect even without a JSON parser on hand.
    let trimmed = stdout.trim();
    assert!(trimmed.ends_with('}'), "stdout must be a single JSON object, got: {stdout}");
    assert!(!trimmed.contains("NaN"), "stdout must not contain a bare NaN token: {stdout}");
    assert!(
        !trimmed.contains(":inf") && !trimmed.contains(":-inf"),
        "stdout must not contain a bare inf token: {stdout}"
    );
}

#[test]
fn missing_input_directory_exits_2() {
    let d = tmpdir("missing");
    let o = Command::new(bin())
        .args(["index", "build", "--input", "/nonexistent/nowhere", "--out"])
        .arg(d.join("x.psidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "usage errors exit 2");
}

#[test]
fn declination_limits_shrink_the_index() {
    let d = tmpdir("dec");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    let build = |tag: &str, extra: &[&str]| -> u64 {
        let out = d.join(format!("{tag}.psidx"));
        let o = Command::new(bin())
            .args(["index", "build", "--input"])
            .arg(&input)
            .arg("--out")
            .arg(&out)
            .args(["--max-mag", "20"])
            .args(extra)
            .output()
            .unwrap();
        assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
        psolve_index::reader::Index::open(&out).unwrap().header().n_records
    };

    // Sample decs are 0.00562, 0.02105, 0.01988.
    assert_eq!(build("all", &[]), 3);
    assert_eq!(build("north", &["--min-dec", "0.01"]), 2);
    assert_eq!(build("south", &["--max-dec", "0.01"]), 1);
}

#[test]
fn a_non_gaia_catalogue_builds_via_column_overrides() {
    let d = tmpdir("columns");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("vizier.csv"),
        "RAJ2000,DEJ2000,Vmag,pmRA,pmDE\n120.5,-33.25,8.75,-4.5,6.25\n10.0,5.0,9.5,0,0\n",
    )
    .unwrap();
    let out = d.join("v.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args([
            "--columns",
            "ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE",
            "--epoch",
            "1991.25",
            "--max-mag",
            "20",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let idx = psolve_index::reader::Index::open(&out).unwrap();
    assert_eq!(idx.header().n_records, 2);
    assert_eq!(idx.header().epoch, 1991.25, "--epoch must reach the header");
}

#[test]
fn bad_column_override_exits_2() {
    let d = tmpdir("badcol");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--columns", "magnitude=Vmag"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn impossible_declination_range_exits_2() {
    let d = tmpdir("badrange");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--min-dec", "40", "--max-dec", "-40"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn building_deeper_than_the_mirror_exits_2() {
    let d = tmpdir("mirror");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    // "files":1 matches the single a.csv actually written above -- this test
    // is about the depth/range checks below, not the fetch-completeness
    // check, so the file count must not itself trip a refusal.
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":14,"min_dec":-90,"max_dec":45,"files":1}"#,
    )
    .unwrap();

    // Deeper than the mirror holds -> refuse rather than build a short index.
    let deep = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("deep.psidx"))
        .args(["--max-mag", "16"])
        .output()
        .unwrap();
    assert_eq!(deep.status.code(), Some(2));

    // Wider declination than the mirror holds -> also refuse.
    let wide = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("wide.psidx"))
        .args(["--max-mag", "14", "--max-dec", "80"])
        .output()
        .unwrap();
    assert_eq!(wide.status.code(), Some(2));

    // Within the mirror -> builds.
    let ok = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("ok.psidx"))
        .args(["--max-mag", "14", "--max-dec", "45"])
        .output()
        .unwrap();
    assert!(ok.status.success(), "stderr: {}", String::from_utf8_lossy(&ok.stderr));
}

#[test]
fn compressed_shards_are_reported_not_silently_skipped() {
    let d = tmpdir("gz");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    // A .gz we cannot decode, and no plain .csv at all.
    std::fs::write(input.join("GaiaSource_000000-003111.csv.gz"), b"\x1f\x8b\x08junk").unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "must not report success with nothing readable");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("compressed") || err.contains("gzip"),
        "stderr should name the compressed files as the reason, got: {err}"
    );
}

#[test]
fn a_mixed_directory_of_csv_and_compressed_files_also_refuses_to_build() {
    // A readable .csv sitting right next to a .csv.gz is the dangerous case:
    // there IS enough input to produce a plausible-looking (but silently
    // short) index, since the .gz would previously be warned about and then
    // ignored while the build still exited 0. It must refuse exactly like
    // the compressed-only case above.
    let d = tmpdir("gz-mixed");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("GaiaSource_000000-003111.csv"), SAMPLE).unwrap();
    std::fs::write(input.join("GaiaSource_003112-005263.csv.gz"), b"\x1f\x8b\x08junk").unwrap();
    let out = d.join("x.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(2),
        "a mixed dir with a readable .csv must still refuse, not build a short index; stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.exists(), "must not write an index when compressed input was present");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("compressed") || err.contains("gzip"),
        "stderr should name the compressed files as the reason, got: {err}"
    );
}

#[test]
fn invalid_nside_exits_2() {
    let d = tmpdir("nside");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--nside", "63"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn non_finite_epoch_exits_2() {
    let d = tmpdir("epoch-nan");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    for bad in ["NaN", "inf", "-inf"] {
        let o = Command::new(bin())
            .args(["index", "build", "--input"])
            .arg(&input)
            .arg("--out")
            .arg(d.join("x.psidx"))
            .args(["--max-mag", "20", "--epoch", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "--epoch {bad} must be rejected");
    }
}

#[test]
fn a_quote_in_name_exits_2_instead_of_corrupting_the_json() {
    let d = tmpdir("name-quote");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--max-mag", "20", "--name", "foo\"bar"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn non_finite_max_mag_exits_2() {
    // Pins the earlier --max-mag fix: RowFilter::validate() bounds-checks
    // declination but has no opinion on magnitude, so a non-finite value here
    // would otherwise reach the filter and silently build a zero-record index.
    let d = tmpdir("max-mag-nan");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    for bad in ["NaN", "inf", "-inf"] {
        let o = Command::new(bin())
            .args(["index", "build", "--input"])
            .arg(&input)
            .arg("--out")
            .arg(d.join("x.psidx"))
            .args(["--max-mag", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "--max-mag {bad} must be rejected");
    }
}

#[test]
fn malformed_jobs_exits_2() {
    // Pins the earlier --jobs fix: a bad value must not be silently swallowed
    // in favour of the default core count.
    let d = tmpdir("jobs-bad");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--max-mag", "20", "--jobs", "banana"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_malformed_mirror_json_refuses_to_build() {
    let d = tmpdir("mirror-bad");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    // Truncated mid-write: present, but not parseable -- must NOT be treated
    // the same as "no mirror.json at all".
    std::fs::write(input.join("mirror.json"), r#"{"max_mag":14,"min_d"#).unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "14"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(!out.exists(), "must not build with an unreadable mirror.json");
}

#[test]
fn no_mirror_json_still_builds() {
    // Proves Absent and Unreadable stayed distinct: a bring-your-own
    // directory with no mirror.json at all must build normally.
    let d = tmpdir("mirror-absent");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out.exists());
}

fn write_two_shards_one_with_a_malformed_row(dir: &std::path::Path) {
    std::fs::write(
        dir.join("good.csv"),
        "ra,dec,pmra,pmdec,phot_g_mean_mag\n\
         10.0,20.0,1.0,1.0,12.0\n\
         11.0,21.0,2.0,2.0,13.0\n",
    )
    .unwrap();
    // The first row parses fine and is kept; the second is malformed (a
    // proper motion of "N/A" is corrupt input, not a legitimate missing
    // value) and aborts the rest of this file's parse, so the third row is
    // never read at all.
    std::fs::write(
        dir.join("bad.csv"),
        "ra,dec,pmra,pmdec,phot_g_mean_mag\n\
         30.0,40.0,1.0,1.0,12.0\n\
         31.0,41.0,N/A,1.0,13.0\n\
         32.0,42.0,1.0,1.0,14.0\n",
    )
    .unwrap();
}

#[test]
fn a_malformed_row_truncates_its_shard_and_is_reported_not_silent() {
    let d = tmpdir("malformed-row");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_two_shards_one_with_a_malformed_row(&input);
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(3),
        "a malformed row that truncates a shard must not exit 0; stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("\"files_failed\":1"), "stdout: {stdout}");
    // good.csv's 2 rows + bad.csv's 1 row before the malformed line = 3;
    // the two rows after the bad line in bad.csv must be gone, not just
    // unreported.
    assert!(
        stdout.contains("\"n_records\":3"),
        "rows parsed before the bad line must still be kept: {stdout}"
    );
    assert!(out.exists(), "the build still completes for what it managed to parse");
}

#[test]
fn allow_partial_turns_a_files_failed_build_into_exit_0() {
    let d = tmpdir("malformed-row-allow");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_two_shards_one_with_a_malformed_row(&input);
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--allow-partial"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("\"files_failed\":1"),
        "--allow-partial silences the exit code, not the report: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_opened_counts_as_a_failed_file_too() {
    use std::os::unix::fs::PermissionsExt;

    let d = tmpdir("unreadable");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("good.csv"), SAMPLE).unwrap();
    let bad = input.join("bad.csv");
    std::fs::write(&bad, SAMPLE).unwrap();
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();

    // Restore permissions regardless of the assertion outcome so temp cleanup
    // (and the wider test process) never trips over a 0-permission file.
    let restore = std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644));

    if !nix_running_as_root() {
        assert_eq!(
            o.status.code(),
            Some(3),
            "an unopenable file must not be silently skipped; stderr: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let stdout = String::from_utf8_lossy(&o.stdout);
        assert!(stdout.contains("\"files_failed\":1"), "stdout: {stdout}");
    }
    restore.unwrap();
}

#[cfg(unix)]
fn nix_running_as_root() -> bool {
    // root can open a 0-permission file, which would make the assertions
    // above meaningless rather than wrong -- skip them in that case (e.g. a
    // container running tests as root) instead of asserting a false failure.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[test]
fn incomplete_mirror_manifest_refuses_to_build() {
    let d = tmpdir("incomplete");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    // Mirrors what fetch-gaia.sh now writes BEFORE the fetch runs: present,
    // parseable, but explicitly not complete yet.
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":20,"min_dec":-90,"max_dec":90,"files":1,"complete":false}"#,
    )
    .unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(3),
        "an interrupted fetch must refuse, not build a silently short index; stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!out.exists());
}

#[test]
fn mirror_file_count_short_of_the_manifest_refuses_to_build() {
    let d = tmpdir("filecount");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    // mirror.json claims 2 files were fetched but only 1 .csv is present --
    // exactly what an interrupted fetch (killed mid-xargs, before the
    // manifest's completion rewrite) leaves behind even under the new
    // before/after manifest scheme, if killed a second time during the
    // in-between window.
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":20,"min_dec":-90,"max_dec":90,"files":2}"#,
    )
    .unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(!out.exists());
}

#[test]
fn allow_partial_builds_from_an_incomplete_mirror() {
    let d = tmpdir("incomplete-allow");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":20,"min_dec":-90,"max_dec":90,"files":2,"complete":false}"#,
    )
    .unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--allow-partial"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out.exists());
}

#[test]
fn a_manifest_without_a_complete_field_still_validates_via_the_file_count() {
    // The manifest an OLD fetch-gaia.sh writes (files, no complete key) must
    // still validate normally when the file count matches -- "complete"
    // missing defaults to true, and the file-count check is the real guard.
    let d = tmpdir("no-complete-field");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":20,"min_dec":-90,"max_dec":90,"files":1}"#,
    )
    .unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn builds_from_a_reduced_shard_with_gaias_null_sentinel_and_keeps_null_pm_rows() {
    // The real shape fetch-gaia.sh writes: 5 columns, no source_id, and
    // Gaia's literal "null" for missing values -- not the 18-column ECSV
    // shape SAMPLE uses. There was previously no end-to-end CLI coverage of
    // the format the tool is actually pointed at in practice.
    let d = tmpdir("null-shard");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("GaiaSource_000000-000001.csv"),
        "ra,dec,pmra,pmdec,phot_g_mean_mag\n\
         10.0,20.0,null,null,12.5\n\
         11.0,21.0,1.5,2.5,13.0\n\
         12.0,22.0,null,null,null\n",
    )
    .unwrap();
    let out = d.join("x.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        stdout.contains("\"n_records\":2"),
        "the null-magnitude row must be skipped but the null-PM rows kept: {stdout}"
    );
    assert!(stdout.contains("\"files_failed\":0"));

    let idx = psolve_index::reader::Index::open(&out).unwrap();
    assert_eq!(idx.header().n_records, 2);
    let stars = idx.brightest_in_disc(10.0, 20.0, 0.01, 10);
    assert_eq!(stars.len(), 1, "the null-PM star must be retained in the index");
    assert_eq!(stars[0].pmra_mas_yr(), 0.0, "a null pmra must decode to zero, not be dropped");
    assert_eq!(stars[0].pmdec_mas_yr(), 0.0);
}

#[test]
fn jobs_zero_exits_2() {
    // rayon treats num_threads(0) as "use the default", which would launder
    // a bogus --jobs 0 into the default thread count rather than rejecting it.
    let d = tmpdir("jobs-zero");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--max-mag", "20", "--jobs", "0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn columns_override_without_an_explicit_name_gets_a_neutral_default_name() {
    let d = tmpdir("neutral-name");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("vizier.csv"),
        "RAJ2000,DEJ2000,Vmag,pmRA,pmDE\n120.5,-33.25,8.75,-4.5,6.25\n",
    )
    .unwrap();
    let out = d.join("v.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args([
            "--columns",
            "ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE",
            "--max-mag",
            "20",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let idx = psolve_index::reader::Index::open(&out).unwrap();
    let name = idx.header().name_str();
    assert!(
        !name.starts_with("gaia-dr3"),
        "a non-Gaia catalogue via --columns must not get a Gaia-branded default name, got {name}"
    );
    assert!(name.starts_with("catalog-"), "got {name}");
}
