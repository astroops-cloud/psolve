use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

const SAMPLE: &str = include_str!("../../psolve-index/tests/fixtures/gaia_sample.csv");

fn built_index(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("psolve-info-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("i.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--name", "info-test"])
        .output()
        .unwrap();
    assert!(o.status.success());
    (d, out)
}

#[test]
fn info_reports_the_header_as_json() {
    let (_d, idx) = built_index("basic");
    let o = Command::new(bin()).args(["index", "info"]).arg(&idx).output().unwrap();
    assert!(o.status.success());
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.trim_start().starts_with('{'), "stdout must be JSON: {s}");
    for key in ["\"name\"", "\"nside\"", "\"n_records\"", "\"epoch\"", "\"sha256\"", "\"occupied_cells\""] {
        assert!(s.contains(key), "missing {key} in {s}");
    }
    assert!(s.contains("info-test"));
}

#[test]
fn digest_ok_is_null_not_false_when_verify_was_not_requested() {
    // Without --verify the digest was never checked. `false` would read as
    // "checked and failed"; `null` correctly says "not checked".
    let (_d, idx) = built_index("no-verify");
    let o = Command::new(bin()).args(["index", "info"]).arg(&idx).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(
        s.contains("\"digest_ok\":null"),
        "digest_ok must be null when --verify was not passed, got: {s}"
    );
    assert!(s.contains("\"verified\":false"));
}

#[test]
fn info_verify_passes_on_a_good_index() {
    let (_d, idx) = built_index("verify-ok");
    let o = Command::new(bin())
        .args(["index", "info", "--verify"])
        .arg(&idx)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"digest_ok\":true"));
}

#[test]
fn info_verify_fails_on_a_corrupted_index_with_exit_3() {
    let (_d, idx) = built_index("verify-bad");
    let mut b = std::fs::read(&idx).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0xff;
    std::fs::write(&idx, &b).unwrap();
    let o = Command::new(bin())
        .args(["index", "info", "--verify"])
        .arg(&idx)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "index problems exit 3");
}

#[test]
fn a_failed_verify_writes_nothing_to_stdout() {
    // Blessed contract: stdout carries results only. A failure emits nothing
    // there -- exit 3 is the signal -- matching every failure path in `build`.
    let (_d, idx) = built_index("verify-silent");
    let mut b = std::fs::read(&idx).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0xff;
    std::fs::write(&idx, &b).unwrap();
    let o = Command::new(bin())
        .args(["index", "info", "--verify"])
        .arg(&idx)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
    assert!(
        o.stdout.is_empty(),
        "stdout must be empty on a failed verify, got: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(!o.stderr.is_empty(), "the reason belongs on stderr");
}

#[test]
fn info_on_a_missing_file_exits_3() {
    let o = Command::new(bin())
        .args(["index", "info", "/nonexistent/none.psidx"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn info_without_a_path_exits_2() {
    let o = Command::new(bin()).args(["index", "info"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}
