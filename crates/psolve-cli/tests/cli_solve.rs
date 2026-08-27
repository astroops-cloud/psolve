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
    let d = std::env::temp_dir().join(format!("psolve-solve-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Minimal valid FITS with no stars.
fn blank_fits(path: &std::path::Path) {
    let cards = [
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    2",
        "NAXIS1  =                   64",
        "NAXIS2  =                   64",
        "BZERO   =                32768",
    ];
    let mut s = String::new();
    for c in cards {
        s.push_str(&format!("{c:<80}"));
    }
    s.push_str(&format!("{:<80}", "END"));
    while !s.len().is_multiple_of(2880) {
        s.push(' ');
    }
    let mut out = s.into_bytes();
    out.extend(std::iter::repeat_n(0u8, 64 * 64 * 2));
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    std::fs::write(path, out).unwrap();
}

/// A tiny but valid index, built through the public CLI so the test exercises
/// the same path an operator would.
fn make_index(d: &std::path::Path) -> std::path::PathBuf {
    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for i in 0..200 {
        let t = i as f64;
        csv.push_str(&format!(
            "{:.6},{:.6},0,0,{:.2}\n",
            (100.0 + (t * 0.013) % 1.5),
            (20.0 + (t * 0.011) % 1.0),
            10.0 + (i % 40) as f64 * 0.1
        ));
    }
    std::fs::write(input.join("a.csv"), csv).unwrap();
    let out = d.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));
    out
}

#[test]
fn a_frame_that_does_not_solve_exits_1_not_2_or_3() {
    // Clouds are a normal outcome, not a broken invocation.
    let d = tmpdir("nosolve");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn a_failed_solve_still_emits_one_valid_json_object_with_a_reason() {
    let d = tmpdir("reason");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    let t = s.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "stdout was: {s}");
    assert!(t.contains("\"solved\":false"));
    assert!(t.contains("\"reason\":\""), "a failure must name its reason: {s}");
    assert!(!t.contains("NaN") && !t.contains(":inf"), "invalid JSON tokens: {s}");
    assert_eq!(t.matches("\"solved\"").count(), 1, "exactly one result object");
    // Spec §7.2: `build` is present on every result, and `index` must now
    // appear on a failure that had one resolved, not only on success -- the
    // fix for the half of the incident where a consumer keyed provenance on
    // `index` and misclassified failure-branch samples that never carried
    // it.
    assert!(t.contains("\"build\":\""), "build must be present on a failure too: {s}");
    assert!(t.contains("\"index\":{\"name\":"), "index must be present on a failure: {s}");
}

/// Reproduces the exact defect verbatim: no `--hint`, and `blank_fits`'s
/// header carries neither `OBJCTRA`/`OBJCTDEC` nor `RA`/`DEC` -- there is
/// genuinely no hint anywhere. This must report `NO_HINT`, not
/// `FOV_MISMATCH`: a caller branching on `reason` must not be told the field
/// of view disagreed when in fact no hint was ever supplied.
#[test]
fn a_hintless_invocation_reports_no_hint_not_fov_mismatch() {
    let d = tmpdir("no-hint");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(s.contains("\"reason\":\"NO_HINT\""), "stdout was: {s}");
    assert!(!s.contains("FOV_MISMATCH"), "stdout was: {s}");
    assert!(
        s.contains("RA/DEC") && s.contains("OBJCTRA"),
        "detail must mention the newly-supported RA/DEC keys too: {s}"
    );
}

#[test]
fn a_missing_file_is_a_usage_error_not_an_index_error() {
    let d = tmpdir("missing");
    let idx = make_index(&d);
    let o = Command::new(bin())
        .args(["solve", "/nonexistent/none.fits", "--index"])
        .arg(&idx)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_missing_index_exits_3() {
    let d = tmpdir("noindex");
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .args(["--index", "/nonexistent/none.psidx", "--hint", "100.0,20.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn no_index_argument_is_a_usage_error() {
    let d = tmpdir("noarg");
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin()).args(["solve"]).arg(&f).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_malformed_hint_is_a_usage_error() {
    let d = tmpdir("badhint");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for bad in ["notanumber", "100.0", "NaN,20.0", "100.0,20.0,30.0"] {
        let o = Command::new(bin())
            .args(["solve"])
            .arg(&f)
            .arg("--index")
            .arg(&idx)
            .args(["--hint", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "hint {bad:?} should be rejected");
    }
}

#[test]
fn progress_goes_to_stderr_and_never_pollutes_stdout() {
    let d = tmpdir("streams");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert_eq!(s.lines().filter(|l| !l.trim().is_empty()).count(), 1,
        "stdout must be exactly one JSON line, got: {s}");
}

#[test]
fn a_flag_value_is_not_mistaken_for_the_file() {
    // --index's value does not start with "--" either, so a naive positional
    // scan binds it as the frame, never reads the real file, and reports a
    // clean "not solved". A broken invocation must not look like weather.
    let d = tmpdir("argorder");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);

    // Flags before the file must behave identically to the file first.
    let flags_first = Command::new(bin())
        .args(["solve", "--index"]).arg(&idx)
        .args(["--hint", "100.0,20.0"]).arg(&f)
        .output().unwrap();
    let file_first = Command::new(bin())
        .args(["solve"]).arg(&f)
        .arg("--index").arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output().unwrap();

    assert_eq!(flags_first.status.code(), file_first.status.code(),
        "argument order must not change the exit code\nflags-first stdout: {}\nfile-first stdout: {}",
        String::from_utf8_lossy(&flags_first.stdout),
        String::from_utf8_lossy(&file_first.stdout));
    let s = String::from_utf8_lossy(&flags_first.stdout);
    assert!(!s.contains("CANNOT_READ"),
        "the real frame must be read regardless of argument order, got: {s}");
}

#[test]
fn a_malformed_radius_is_a_usage_error() {
    // A mistyped search radius must not silently become the default 2.5 deg
    // search -- that turns a broken invocation into a clean "not solved",
    // exactly the Task 13 defect in a new costume.
    let d = tmpdir("badradius");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for bad in ["not-a-number", "-5", "0", "NaN"] {
        let o = Command::new(bin())
            .args(["solve"])
            .arg(&f)
            .arg("--index")
            .arg(&idx)
            .args(["--hint", "100.0,20.0"])
            .args(["--radius", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "radius {bad:?} should be rejected");
    }
}

#[test]
fn a_malformed_cat_limit_is_a_usage_error() {
    let d = tmpdir("badcatlimit");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for bad in ["lots", "-5", "0", "3.5"] {
        let o = Command::new(bin())
            .args(["solve"])
            .arg(&f)
            .arg("--index")
            .arg(&idx)
            .args(["--hint", "100.0,20.0"])
            .args(["--cat-limit", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "cat-limit {bad:?} should be rejected");
    }
}

/// A malformed `--max-mag` is a usage error, and a well-formed one must not
/// be mistaken for the positional FILE.
///
/// The second half is the one that has bitten before: an unregistered valued
/// flag makes the positional scan bind the flag's VALUE as the input file,
/// which then reports a clean exit-1 "not solved" for what was really a
/// broken invocation.
#[test]
fn max_mag_is_validated_and_never_bound_as_the_input_file() {
    let d = tmpdir("badmaxmag");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for bad in ["bright", "nan", "inf"] {
        let o = Command::new(bin())
            .args(["solve"])
            .arg(&f)
            .arg("--index")
            .arg(&idx)
            .args(["--hint", "100.0,20.0"])
            .args(["--max-mag", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "max-mag {bad:?} should be rejected");
    }
    // Flag BEFORE the positional: its value must not become the frame path.
    let o = Command::new(bin())
        .args(["solve"])
        .args(["--max-mag", "12.0"])
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .arg(&f)
        .output()
        .unwrap();
    assert_ne!(
        o.status.code(),
        Some(2),
        "--max-mag's value was bound as the input file: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn a_malformed_saturation_is_a_usage_error() {
    let d = tmpdir("badsaturation");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for bad in ["not-a-number", "-5", "0", "NaN"] {
        let o = Command::new(bin())
            .args(["solve"])
            .arg(&f)
            .arg("--index")
            .arg(&idx)
            .args(["--hint", "100.0,20.0"])
            .args(["--saturation", bad])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "saturation {bad:?} should be rejected");
    }
}

#[test]
fn every_valued_flag_can_precede_the_file() {
    let d = tmpdir("argorder2");
    let idx = make_index(&d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    for extra in [
        vec!["--scale", "2.4614"],
        vec!["--radius", "2.0"],
        vec!["--cat-limit", "500"],
        vec!["--saturation", "50000"],
    ] {
        let o = Command::new(bin())
            .args(["solve", "--index"]).arg(&idx)
            .args(&extra).arg(&f)
            .args(["--hint", "100.0,20.0"])
            .output().unwrap();
        let s = String::from_utf8_lossy(&o.stdout);
        assert!(!s.contains("CANNOT_READ"),
            "{extra:?} before FILE broke the positional scan: {s}");
    }
}
