// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! DC-7/D2+D3: `mediagit key`, and what a keyed repository does end to end.
//!
//! Every test here pins `MEDIAGIT_ENCRYPTION_KEYFILE` at a throwaway file.
//! That is not only for determinism: without it, `key init` would provision a
//! master key into the *developer's real OS keychain*, and a test suite has no
//! business writing to that.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A 32-byte master key on disk, in the hex form `openssl rand -hex 32` emits.
fn write_keyfile(dir: &Path, byte: u8) -> PathBuf {
    let path = dir.join("master.key");
    fs::write(&path, hex_of(byte)).unwrap();
    path
}

fn hex_of(byte: u8) -> String {
    (0..32).map(|_| format!("{byte:02x}")).collect()
}

fn mediagit(keyfile: Option<&Path>) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        // The credential keychain is a separate concern, but the same rule
        // applies: tests must not touch the machine's real one.
        .env("MEDIAGIT_NO_KEYRING", "1");
    match keyfile {
        Some(p) => c.env("MEDIAGIT_ENCRYPTION_KEYFILE", p),
        None => c.env_remove("MEDIAGIT_ENCRYPTION_KEYFILE"),
    };
    c
}

fn init_repo(dir: &Path) {
    mediagit(None)
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Every regular file under `.mediagit/objects`, so a test can look at what
/// was actually written rather than trusting the command's own report.
fn object_files(repo: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo.join(".mediagit").join("objects"), &mut out);
    out
}

/// Does any object on disk carry the MGEN envelope magic?
fn any_object_is_sealed(repo: &Path) -> bool {
    object_files(repo)
        .iter()
        .filter_map(|p| fs::read(p).ok())
        .any(|bytes| bytes.starts_with(b"MGEN"))
}

#[test]
fn key_status_reports_off_for_an_ordinary_repository() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    mediagit(None)
        .arg("key")
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("At-rest encryption: off"));

    // Absence of a key must be completely silent everywhere else — not a
    // warning, not a prompt. An ordinary command in an unencrypted repo says
    // nothing about encryption at all.
    mediagit(None)
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("encryption").not());
}

#[test]
fn key_init_refuses_to_overwrite_an_existing_key() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0xA1);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();

    let before = fs::read(repo.join(".mediagit").join("encryption-key")).unwrap();

    // The single most destructive mistake this command could make. A second
    // key would orphan every object already sealed under the first, and
    // nothing in the repository records which key an object used.
    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already has an encryption key"));

    assert_eq!(
        before,
        fs::read(repo.join(".mediagit").join("encryption-key")).unwrap(),
        "the refused `key init` must not have touched the existing key file"
    );
}

#[test]
fn key_status_never_prints_key_material() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0xC3);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();

    let out = mediagit(Some(&keyfile))
        .arg("key")
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success()
        .get_output()
        .clone();
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(rendered.contains("At-rest encryption: on"));
    assert!(rendered.contains("key file named by MEDIAGIT_ENCRYPTION_KEYFILE"));
    assert!(
        !rendered.contains(&hex_of(0xC3)),
        "status must never echo the master key"
    );
    // Nor the wrapped repo key, which is ciphertext but still key material.
    let stored = fs::read_to_string(repo.join(".mediagit").join("encryption-key")).unwrap();
    let wrapped = stored
        .lines()
        .find_map(|l| l.strip_prefix("wrapped = "))
        .unwrap()
        .trim_matches('"');
    assert!(
        !rendered.contains(wrapped),
        "status must never echo the wrapped key"
    );
}

/// The end-to-end shape of D3: after `key init`, ordinary commands seal what
/// they write and read it back, with no per-command flag anywhere.
#[test]
fn a_keyed_repository_seals_objects_and_still_reads_them() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x5E);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();

    fs::write(repo.join("asset.txt"), "sealed payload ".repeat(200)).unwrap();
    mediagit(Some(&keyfile))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("commit")
        .arg("-m")
        .arg("encrypted commit")
        .current_dir(&repo)
        .assert()
        .success();

    assert!(
        any_object_is_sealed(&repo),
        "with a key installed, objects on disk must carry the MGEN envelope"
    );

    // And the write is readable — the round trip, not just the seal.
    mediagit(Some(&keyfile))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("encrypted commit"));

    // Content, not just metadata: restore the file from sealed objects.
    fs::remove_file(repo.join("asset.txt")).unwrap();
    mediagit(Some(&keyfile))
        .arg("reset")
        .arg("--hard")
        .arg("HEAD")
        .current_dir(&repo)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(repo.join("asset.txt")).unwrap(),
        "sealed payload ".repeat(200),
        "content did not survive the seal/open round trip"
    );
}

/// Fail closed. Reading sealed objects with the wrong master key — or none —
/// must be an error, never bytes: the codec sniffer downstream would take
/// AES-GCM output for some codec and hand a caller random bytes as content.
#[test]
fn a_keyed_repository_fails_closed_without_the_right_master_key() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x2B);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();
    fs::write(repo.join("asset.txt"), "secret").unwrap();
    mediagit(Some(&keyfile))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("commit")
        .arg("-m")
        .arg("secret commit")
        .current_dir(&repo)
        .assert()
        .success();

    // Wrong master key.
    let wrong = dir.path().join("wrong.key");
    fs::write(&wrong, hex_of(0x77)).unwrap();
    mediagit(Some(&wrong))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("could not unlock"));

    // `key status` must still work with the wrong key — reporting on a repo
    // you cannot open is exactly when you need it most.
    mediagit(Some(&wrong))
        .arg("key")
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("At-rest encryption: on"));
}

/// Enabling encryption must not orphan what the repository already holds.
#[test]
fn objects_written_before_key_init_stay_readable() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x11);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    fs::write(repo.join("old.txt"), "written in the clear ".repeat(50)).unwrap();
    mediagit(None)
        .arg("add")
        .arg("old.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(None)
        .arg("commit")
        .arg("-m")
        .arg("plaintext commit")
        .current_dir(&repo)
        .assert()
        .success();

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();

    // Same objects, now read by a keyed process.
    mediagit(Some(&keyfile))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("plaintext commit"));

    fs::remove_file(repo.join("old.txt")).unwrap();
    mediagit(Some(&keyfile))
        .arg("reset")
        .arg("--hard")
        .arg("HEAD")
        .current_dir(&repo)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(repo.join("old.txt")).unwrap(),
        "written in the clear ".repeat(50),
        "a keyed repo must still read objects written before the key existed"
    );
}

/// Pull the recovery code out of `key init`'s stdout, the way a user copies it
/// off their terminal.
fn recovery_code_from(stdout: &str) -> String {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| l.len() == 71 && l.matches('-').count() == 7)
        .unwrap_or_else(|| panic!("no recovery code in `key init` output:\n{stdout}"))
        .to_string()
}

fn key_init(repo: &Path, keyfile: &Path) -> String {
    let out = mediagit(Some(keyfile))
        .arg("key")
        .arg("init")
        .current_dir(repo)
        .assert()
        .success()
        .get_output()
        .clone();
    recovery_code_from(&String::from_utf8_lossy(&out.stdout))
}

/// The whole point of the second slot: the master key is gone, and the code
/// gets you back in — including to objects written before the recovery.
#[test]
fn a_recovery_code_unlocks_a_repo_whose_master_key_is_lost() {
    let dir = TempDir::new().unwrap();
    let original = write_keyfile(dir.path(), 0x3C);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let code = key_init(&repo, &original);

    let content = "recovered payload ".repeat(120);
    fs::write(repo.join("asset.txt"), &content).unwrap();
    mediagit(Some(&original))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&original))
        .arg("commit")
        .arg("-m")
        .arg("written before recovery")
        .current_dir(&repo)
        .assert()
        .success();

    // The master key is lost. A different one cannot open the repo.
    let replacement = dir.path().join("replacement.key");
    fs::write(&replacement, hex_of(0xE7)).unwrap();
    mediagit(Some(&replacement))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .failure();

    // Recover: re-wraps the same repo key under the master source available now.
    mediagit(Some(&replacement))
        .arg("key")
        .arg("recover")
        .arg(&code)
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository unlocked"));

    // A real round-trip, not a key comparison: objects written before the
    // recovery must still decrypt and still match byte for byte.
    mediagit(Some(&replacement))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("written before recovery"));

    fs::remove_file(repo.join("asset.txt")).unwrap();
    mediagit(Some(&replacement))
        .arg("reset")
        .arg("--hard")
        .arg("HEAD")
        .current_dir(&repo)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(repo.join("asset.txt")).unwrap(),
        content,
        "recovery must yield the same repo key, not merely a plausible one"
    );

    // The code is carried over, so recovery is repeatable rather than a
    // one-shot that quietly burns the user's last way in.
    mediagit(Some(&original))
        .arg("key")
        .arg("recover")
        .arg(&code)
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&original))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .success();
}

/// A mistyped code must fail closed and change nothing.
#[test]
fn a_wrong_recovery_code_is_refused_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x6A);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let code = key_init(&repo, &keyfile);
    let key_file = repo.join(".mediagit").join("encryption-key");
    let before = fs::read(&key_file).unwrap();

    // One character off — same shape, same length.
    let mut wrong: Vec<char> = code.chars().collect();
    let pos = wrong.iter().position(|c| c.is_ascii_hexdigit()).unwrap();
    wrong[pos] = if wrong[pos] == 'a' { 'b' } else { 'a' };
    let wrong: String = wrong.into_iter().collect();

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("recover")
        .arg(&wrong)
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not open this repository"));

    assert_eq!(
        before,
        fs::read(&key_file).unwrap(),
        "a refused recovery must not have rewritten the key file"
    );

    // Malformed input is a message too, not a panic.
    mediagit(Some(&keyfile))
        .arg("key")
        .arg("recover")
        .arg("obviously-not-a-code")
        .current_dir(&repo)
        .assert()
        .failure();
}

/// `key status` must say whether a second way in exists, without being one.
#[test]
fn key_status_reports_the_recovery_slot_without_leaking_it() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x8F);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let code = key_init(&repo, &keyfile);

    let out = mediagit(Some(&keyfile))
        .arg("key")
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success()
        .get_output()
        .clone();
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(rendered.contains("Recovery slot: present"));
    assert!(
        !rendered.contains(&code) && !rendered.contains(&code.replace('-', "")),
        "status must never echo the recovery code"
    );

    // And the code is nowhere on disk in unwrapped form.
    let stored = fs::read_to_string(repo.join(".mediagit").join("encryption-key")).unwrap();
    assert!(
        !stored.contains(&code.replace('-', "")),
        "the recovery code must never be written to the key file"
    );
    assert!(stored.contains("recovery = "));
}

/// A key file predating recovery slots must load exactly as it did, and say so
/// rather than implying a fallback that is not there.
#[test]
fn a_repo_without_a_recovery_slot_still_works() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x4D);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    key_init(&repo, &keyfile);

    // Strip the slot, reproducing a file written before this feature existed.
    let key_file = repo.join(".mediagit").join("encryption-key");
    let stripped: String = fs::read_to_string(&key_file)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("recovery = "))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&key_file, stripped).unwrap();

    fs::write(repo.join("asset.txt"), "legacy slot layout").unwrap();
    mediagit(Some(&keyfile))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("commit")
        .arg("-m")
        .arg("no recovery slot")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("log")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("no recovery slot"));

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery slot: none"));

    // And `key recover` says why it cannot help, rather than failing obscurely.
    mediagit(Some(&keyfile))
        .arg("key")
        .arg("recover")
        .arg("00000000-00000000-00000000-00000000-00000000-00000000-00000000-00000000")
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("no recovery slot"));
}

/// The push interlock. D4 (client key escrow) does not exist, so push must
/// refuse loudly rather than populate a remote with objects nothing there can
/// verify or hand back.
#[test]
fn push_refuses_on_an_encrypted_repository() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x9D);
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&repo)
        .assert()
        .success();
    fs::write(repo.join("asset.txt"), "payload").unwrap();
    mediagit(Some(&keyfile))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("commit")
        .arg("-m")
        .arg("c1")
        .current_dir(&repo)
        .assert()
        .success();
    mediagit(Some(&keyfile))
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://127.0.0.1:59999/repo")
        .current_dir(&repo)
        .assert()
        .success();

    mediagit(Some(&keyfile))
        .arg("push")
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("at-rest encryption"))
        .stderr(predicate::str::contains("Nothing was uploaded"));
}

/// F3: the at-rest key is a per-repository secret living in a process-global
/// slot, and `find_repo_root()` walks *upward*. So a `clone` run from inside
/// an encrypted repository picks up the outer repo's key and would seal the
/// new repository's objects under it — while the new repository gets no key
/// file of its own, leaving it permanently unopenable.
///
/// The refusal is at the storage boundary, not in `clone`: special-casing
/// `clone` and `init` would patch the two callers anyone has noticed and leave
/// every future one open.
#[test]
fn a_command_inside_an_encrypted_repo_refuses_to_open_a_different_repo() {
    let dir = TempDir::new().unwrap();
    let keyfile = write_keyfile(dir.path(), 0x6D);
    let outer = dir.path().join("outer");
    fs::create_dir_all(&outer).unwrap();
    init_repo(&outer);

    mediagit(Some(&keyfile))
        .arg("key")
        .arg("init")
        .current_dir(&outer)
        .assert()
        .success();

    // The live case. The URL is unreachable on purpose: the guard has to fire
    // before any network or storage I/O, so this must fail on the key scope,
    // not on a connection error.
    let assert = mediagit(Some(&keyfile))
        .arg("clone")
        .arg("http://127.0.0.1:59998/other.git")
        .arg("sub")
        .current_dir(&outer)
        .assert()
        .failure()
        .stderr(predicate::str::contains("different repository"));
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("Connection refused") && !stderr.contains("error sending request"),
        "the guard must precede the network, got:\n{stderr}"
    );

    // Nothing of the aborted clone may have been sealed under the outer key.
    assert!(
        !any_object_is_sealed(&outer.join("sub")),
        "the refused clone must not have written objects into the nested repo"
    );

    // Same class, a different caller: `init` of a nested repository. This is
    // the proof the guard is not a `clone` special case.
    mediagit(Some(&keyfile))
        .arg("init")
        .arg("nested")
        .current_dir(&outer)
        .assert()
        .failure()
        .stderr(predicate::str::contains("different repository"));

    // And the outer repository itself keeps working — a guard that also
    // refuses the repo the key belongs to would be worse than the bug.
    fs::write(outer.join("asset.txt"), "still fine ".repeat(50)).unwrap();
    mediagit(Some(&keyfile))
        .arg("add")
        .arg("asset.txt")
        .current_dir(&outer)
        .assert()
        .success();
}
