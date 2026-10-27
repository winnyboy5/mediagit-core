// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! #94: `status` stat cache. The cache may only make `status` faster; every
//! test here checks that a real change is still reported, or that output is
//! identical with the cache on and off.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env_remove("MEDIAGIT_STAT_CACHE")
        .current_dir(dir);
    c
}

fn set_mtime(path: &Path, t: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn an_hour_ago() -> SystemTime {
    SystemTime::now() - Duration::from_secs(3600)
}

/// Repo with `a.bin` and `b.bin` committed and their mtimes moved an hour
/// into the past, so entries recorded now are not racy and will be trusted.
fn committed_repo() -> TempDir {
    let d = TempDir::new().unwrap();
    let p = d.path();
    mediagit(p).args(["init", "-q"]).assert().success();
    fs::write(p.join("a.bin"), b"hello").unwrap();
    fs::write(p.join("b.bin"), b"world").unwrap();
    mediagit(p)
        .args(["add", "a.bin", "b.bin", "-q"])
        .assert()
        .success();
    mediagit(p)
        .args(["commit", "-m", "init", "-q"])
        .assert()
        .success();
    set_mtime(&p.join("a.bin"), an_hour_ago());
    set_mtime(&p.join("b.bin"), an_hour_ago());
    d
}

fn porcelain(dir: &Path, cache_off: bool) -> String {
    let mut c = mediagit(dir);
    if cache_off {
        c.env("MEDIAGIT_STAT_CACHE", "0");
    }
    let out = c.args(["status", "--porcelain"]).assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

fn cache_file(dir: &Path) -> std::path::PathBuf {
    dir.join(".mediagit").join("stat-cache")
}

#[test]
fn status_writes_cache() {
    let d = committed_repo();
    assert_eq!(porcelain(d.path(), false), "");
    assert!(
        cache_file(d.path()).exists(),
        "status should record the stat cache"
    );
}

#[test]
fn disabled_cache_writes_nothing_and_output_matches() {
    let d = committed_repo();
    fs::write(d.path().join("a.bin"), b"HELLO!").unwrap();
    let off = porcelain(d.path(), true);
    assert!(!cache_file(d.path()).exists());
    let on = porcelain(d.path(), false);
    assert_eq!(on, off);
    assert!(off.contains("a.bin"), "edit must be reported: {off:?}");
}

#[test]
fn edit_after_warm_cache_is_reported() {
    let d = committed_repo();
    let p = d.path();
    porcelain(p, false); // warm
    porcelain(p, false); // served from cache
    // Same-size edit (new mtime) and a size change.
    fs::write(p.join("a.bin"), b"HELLO").unwrap();
    fs::write(p.join("b.bin"), b"world, longer").unwrap();
    let out = porcelain(p, false);
    assert!(out.contains("a.bin") && out.contains("b.bin"), "{out:?}");
    assert_eq!(out, porcelain(p, true));
}

#[test]
fn edit_right_after_commit_is_reported() {
    // Racy case: file recorded in the same tick it was written, then edited
    // keeping its size. The racy rule must force a re-hash.
    let d = TempDir::new().unwrap();
    let p = d.path();
    mediagit(p).args(["init", "-q"]).assert().success();
    fs::write(p.join("a.bin"), b"hello").unwrap();
    mediagit(p).args(["add", "a.bin", "-q"]).assert().success();
    mediagit(p)
        .args(["commit", "-m", "init", "-q"])
        .assert()
        .success();
    let t = fs::metadata(p.join("a.bin")).unwrap().modified().unwrap();
    porcelain(p, false);
    fs::write(p.join("a.bin"), b"HELLO").unwrap();
    set_mtime(&p.join("a.bin"), t);
    assert!(porcelain(p, false).contains("a.bin"));
}

#[test]
fn replace_via_rename_keeping_size_and_mtime_is_reported() {
    let d = committed_repo();
    let p = d.path();
    porcelain(p, false); // warm
    std::thread::sleep(Duration::from_millis(20));
    let tmp = p.join("a.tmp");
    fs::write(&tmp, b"HELLO").unwrap();
    set_mtime(
        &tmp,
        fs::metadata(p.join("a.bin")).unwrap().modified().unwrap(),
    );
    fs::rename(&tmp, p.join("a.bin")).unwrap();
    assert!(porcelain(p, false).contains("a.bin"));
}

#[test]
fn corrupt_cache_is_ignored() {
    let d = committed_repo();
    let p = d.path();
    porcelain(p, false);
    fs::write(cache_file(p), b"\x01not postcard at all").unwrap();
    fs::write(p.join("a.bin"), b"HELLO!").unwrap();
    assert_eq!(porcelain(p, false), porcelain(p, true));
    assert!(porcelain(p, false).contains("a.bin"));
}

/// Documents the known limit (same as Git) and proves the cache is really
/// consulted: on Windows an in-place rewrite that restores size and mtime
/// keeps the creation time, so the cached hash is trusted. The escape hatch
/// `MEDIAGIT_STAT_CACHE=0` still sees the edit.
#[cfg(windows)]
#[test]
fn in_place_rewrite_with_same_size_and_mtime_is_the_documented_limit() {
    let d = committed_repo();
    let p = d.path();
    porcelain(p, false); // warm
    let t = fs::metadata(p.join("a.bin")).unwrap().modified().unwrap();
    fs::write(p.join("a.bin"), b"HELLO").unwrap();
    set_mtime(&p.join("a.bin"), t);
    assert_eq!(
        porcelain(p, false),
        "",
        "cache hit expected (documented limit)"
    );
    assert!(porcelain(p, true).contains("a.bin"));
}

/// Status lists come from hash sets and a parallel walk; output must still
/// be in path order every run (porcelain is a frozen script interface).
#[test]
fn porcelain_lists_are_in_path_order() {
    let d = TempDir::new().unwrap();
    let p = d.path();
    mediagit(p).args(["init", "-q"]).assert().success();
    let names: Vec<String> = (0..40).map(|i| format!("f{i:02}.bin")).collect();
    for n in &names {
        fs::write(p.join(n), n.as_bytes()).unwrap();
    }
    mediagit(p).args(["add", ".", "-q"]).assert().success();
    mediagit(p)
        .args(["commit", "-m", "init", "-q"])
        .assert()
        .success();
    for (i, n) in names.iter().enumerate() {
        match i % 3 {
            0 => fs::write(p.join(n), b"changed").unwrap(),
            1 => fs::remove_file(p.join(n)).unwrap(),
            _ => {}
        }
        fs::write(p.join(format!("new{i:02}.bin")), b"x").unwrap();
    }
    let out = porcelain(p, false);
    for code in [" M ", " D ", "?? "] {
        let paths: Vec<&str> = out.lines().filter_map(|l| l.strip_prefix(code)).collect();
        assert!(!paths.is_empty(), "no {code:?} lines in {out}");
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "{code:?} lines not in path order");
    }
}

/// Every status output mode is identical with the cache on (cold and warm)
/// and off, on a dirty tree and a clean tree.
#[test]
fn all_output_modes_match_with_cache_on_and_off() {
    let d = committed_repo();
    let p = d.path();
    let run = |args: &[&str], off: bool| {
        let mut c = mediagit(p);
        if off {
            c.env("MEDIAGIT_STAT_CACHE", "0");
        }
        let out = c.arg("status").args(args).assert().success();
        String::from_utf8(out.get_output().stdout.clone()).unwrap()
    };
    let modes: [&[&str]; 4] = [&[], &["--short"], &["--porcelain"], &["--json"]];
    for dirty in [false, true] {
        if dirty {
            fs::write(p.join("a.bin"), b"HELLO!").unwrap();
            fs::remove_file(p.join("b.bin")).unwrap();
            fs::write(p.join("c.bin"), b"new").unwrap();
        }
        for args in modes {
            let off = run(args, true);
            assert_eq!(run(args, false), off, "cold cache, {args:?}, dirty={dirty}");
            assert_eq!(run(args, false), off, "warm cache, {args:?}, dirty={dirty}");
        }
    }
}
