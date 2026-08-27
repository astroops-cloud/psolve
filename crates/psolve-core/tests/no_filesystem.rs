//! The structural guarantee: psolve-core cannot touch the filesystem.
//!
//! Spec section 4 makes this a property of the dependency graph rather than of
//! discipline -- a future caller cannot accidentally rewrite catalogue data,
//! because the code that reads an index has no way to write one. A grep test is
//! crude but it fails loudly the moment someone adds `use std::fs`.

use std::path::Path;

fn core_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).expect("psolve-core/src must exist");
    for e in entries {
        let p = e.expect("readable dir entry").path();
        if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
            let text = std::fs::read_to_string(&p).expect("readable source file");
            out.push((name, text));
        }
    }
    assert!(!out.is_empty(), "found no sources to check");
    out
}

/// Split source into identifier tokens: runs of [A-Za-z0-9_].
///
/// Tokenising rather than substring-matching is what makes `path_length` a
/// single token that does NOT match `path`, while `fs::read` still yields the
/// token `fs`. It also removes any need to strip comments -- and comment
/// stripping was itself a bypass, because a `//` inside a string literal hid
/// every following character from the scanner.
fn tokens(src: &str) -> Vec<&str> {
    src.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Reaching outside memory requires one of these names. Deliberately narrow:
/// `Path`/`path` are absent because manipulating a path is not I/O, and
/// including them would false-positive on ordinary geometry code.
///
/// Comments are NOT stripped, so this also fires on prose containing these
/// bare words. That is intentional -- it fails closed, and rewording a comment
/// is cheaper than an undetected filesystem access.
const FORBIDDEN: &[&str] = &[
    "fs", "net", "process", "env", "File", "OpenOptions", "PathBuf",
];

fn violations(src: &str) -> Vec<&'static str> {
    let toks = tokens(src);
    FORBIDDEN
        .iter()
        .copied()
        .filter(|f| toks.iter().any(|t| t == f))
        .collect()
}

#[test]
fn core_never_touches_the_filesystem() {
    for (name, text) in core_sources() {
        let viols = violations(&text);
        assert!(
            viols.is_empty(),
            "{name} contains forbidden patterns {viols:?} -- psolve-core must not reach outside memory. \
             If a caller needs to read a file, it reads it and passes the bytes in."
        );
    }
}

#[test]
fn core_declares_no_dependencies() {
    let toml = include_str!("../Cargo.toml");
    let deps = toml.split("[dependencies]").nth(1).unwrap_or("");
    let real: Vec<&str> = deps
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['))
        .collect();
    assert!(
        real.is_empty(),
        "psolve-core must have no dependencies (including dev), found: {real:?}"
    );
}

#[test]
fn the_guard_catches_the_ways_a_bypass_would_actually_be_written() {
    for bypass in [
        "use std::fs;",
        "use std::{fs, path::Path};",
        "use std::{io, fs};",
        "let d = ::std::fs::read(p);",
        "let f = File::open(p);",
        "use std::process::Command;",
        "let home = std::env::var(\"HOME\");",
        // The masking case: a string containing // must not hide what follows.
        "let m = \"see https://docs.rs/x\"; let d = std::fs::read(p);",
        // ...nor a /*-lookalike inside a string.
        "let m = \"km/*hr\"; let d = std::fs::read(p);",
    ] {
        assert!(!violations(bypass).is_empty(), "guard failed to catch: {bypass}");
    }
}

#[test]
fn the_guard_does_not_fire_on_ordinary_solver_code() {
    for ok in [
        "pub fn decode(bytes: &[u8]) -> Vec<f32> { Vec::new() }",
        // The false positives the previous version produced:
        "fn solve(image: &[f32], path_length: f64) -> f64 { path_length * 2.0 }",
        "fn f(gross: f64, net_flux: f64) -> f64 { gross - net_flux }",
        "fn pipeline(raw: &[u8], process_frame: bool) -> Vec<u8> { Vec::new() }",
        "struct Star { flux: f64, path_length: f64 }",
        "let scale = a / b / c;",
        "let ratio = width / height;",
    ] {
        assert!(violations(ok).is_empty(), "guard false-positived on: {ok}");
    }
}
