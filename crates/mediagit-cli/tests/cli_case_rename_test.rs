// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-011: a case-only rename (`art/Hero.txt` to `art/hero.txt`) must leave
//! exactly one name in the next version, and a repository that already holds
//! both names must have a way out.
//!
//! Tests that need a case-insensitive filesystem probe the temp dir at run
//! time and return early where names are case-sensitive. Tests that plant a
//! two-name tree through the library API do not depend on the filesystem.

use assert_cmd::Command;
use mediagit_versioning::{
    Commit, FileMode, ObjectType, Ref, RefDatabase, Signature, Tree, TreeEntry,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    cmd
}

fn run(root: &Path, args: &[&str]) -> (bool, String) {
    let out = mediagit().args(args).current_dir(root).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn must(root: &Path, args: &[&str]) -> String {
    let (ok, text) = run(root, args);
    assert!(ok, "{args:?} failed: {text}");
    text
}

/// True when `dir` treats `P` and `p` as the same name.
fn case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("case-probe-lower");
    fs::write(&probe, b"").unwrap();
    let same = dir.join("CASE-PROBE-LOWER").exists();
    fs::remove_file(&probe).unwrap();
    same
}

/// Names actually stored in `dir`, as spelled on disk.
fn disk_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn repo_with(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    must(root, &["init", "-q"]);
    for (rel, body) in files {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "one"]);
    dir
}

/// Lines of `ls-files` (tracked names), unfiltered.
fn tracked(root: &Path) -> Vec<String> {
    must(root, &["ls-files"])
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

fn count_name(lines: &[String], name: &str) -> usize {
    lines
        .iter()
        .filter(|l| l.split_whitespace().any(|w| w == name))
        .count()
}

/// A child of HEAD whose tree also holds `extra` (same blob as `like`),
/// planted through the library, bypassing the write-side guard.
async fn plant_second_name(root: &Path, like: &str, extra: &str) {
    plant(
        root,
        |tree| {
            let oid = tree.get_entry(like).unwrap().oid;
            tree.add_entry(TreeEntry::new(extra.to_string(), FileMode::Regular, oid));
        },
        true,
    )
    .await;
}

/// A child of HEAD whose tree is HEAD's tree after `edit`, written through the
/// library (bypassing every write-side guard). Returns the commit id in hex.
/// With `move_head` the current branch moves to it; otherwise nothing moves.
async fn plant(root: &Path, edit: impl FnOnce(&mut Tree), move_head: bool) -> String {
    let storage = mediagit_cli::repo::create_storage_backend(root)
        .await
        .unwrap();
    let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 100);
    let refdb = RefDatabase::new(root.join(".mediagit"));
    let head = refdb.resolve("HEAD").await.unwrap();
    let commit = Commit::deserialize(&odb.read(&head).await.unwrap()).unwrap();
    let mut tree = Tree::deserialize(&odb.read(&commit.tree).await.unwrap()).unwrap();
    edit(&mut tree);
    let tree_oid = odb
        .write(
            ObjectType::Tree,
            &mediagit_versioning::format::serialize(&tree).unwrap(),
        )
        .await
        .unwrap();
    let sig = Signature::now("t".into(), "t@e".into());
    let planted = Commit::with_parents(tree_oid, vec![head], sig.clone(), sig, "both".into());
    // Raw write: `Commit::write` refuses a newly introduced case pair.
    let new_head = odb
        .write(ObjectType::Commit, &planted.serialize().unwrap())
        .await
        .unwrap();
    if move_head {
        let branch = refdb.read("HEAD").await.unwrap().target.unwrap();
        refdb
            .write(&Ref::new_direct(branch, new_head))
            .await
            .unwrap();
    }
    new_head.to_hex()
}

const BAD: &str = "//?/C:/Users/someone/work/a.txt";

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn case_only_rename_commits_one_name_and_reset_restores_it() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    fs::rename(root.join("art/Hero.txt"), root.join("art/hero.txt")).unwrap();

    let out = must(root, &["add", "--all"]);
    assert!(out.contains("1 deletion"), "deletion must be staged: {out}");
    must(root, &["commit", "-m", "rename"]);

    let names = tracked(root);
    assert_eq!(count_name(&names, "art/hero.txt"), 1, "{names:?}");
    assert_eq!(count_name(&names, "art/Hero.txt"), 0, "{names:?}");

    // The whole version checks out and keeps the on-disk casing.
    fs::remove_file(root.join("keep.txt")).unwrap();
    must(root, &["reset", "--hard", "HEAD"]);
    assert_eq!(disk_names(&root.join("art")), vec!["hero.txt"]);
    assert!(root.join("keep.txt").exists());
}

#[test]
fn case_only_folder_rename_commits_one_spelling() {
    let dir = repo_with(&[("art/x.txt", "x"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    fs::rename(root.join("art"), root.join("tmp-art")).unwrap();
    fs::rename(root.join("tmp-art"), root.join("Art")).unwrap();

    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "folder"]);

    let names = tracked(root);
    assert_eq!(count_name(&names, "Art/x.txt"), 1, "{names:?}");
    assert_eq!(count_name(&names, "art/x.txt"), 0, "{names:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repository_holding_both_names_add_all_is_the_way_out() {
    let dir = repo_with(&[("art/hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    // HEAD (from an older release) holds Hero.txt too; disk holds hero.txt.
    plant_second_name(root, "art/hero.txt", "art/Hero.txt").await;

    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "repair"]);

    let names = tracked(root);
    assert_eq!(count_name(&names, "art/hero.txt"), 1, "{names:?}");
    assert_eq!(count_name(&names, "art/Hero.txt"), 0, "{names:?}");

    fs::remove_file(root.join("keep.txt")).unwrap();
    must(root, &["reset", "--hard", "HEAD"]);
    assert_eq!(disk_names(&root.join("art")), vec!["hero.txt"]);
    assert!(root.join("keep.txt").exists());
}

/// Independent of the filesystem: a repository that already holds a case pair
/// (for example made on Linux) keeps working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn existing_case_pair_does_not_block_commits() {
    let dir = repo_with(&[("art/hero.txt", "hero")]);
    let root = dir.path();
    plant_second_name(root, "art/hero.txt", "art/Hero.txt").await;

    // A pair already in the parent version may stay: an unrelated commit works.
    let (ok, text) = run(root, &["commit", "--allow-empty", "-m", "x"]);
    assert!(ok, "an existing pair must not block commits: {text}");
}

/// A commit that would introduce a NEW pair is refused, naming both paths and
/// the way out; HEAD stays put. Built through the index API, any filesystem.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commit_refuses_a_newly_introduced_case_pair() {
    let dir = repo_with(&[("art/hero.txt", "hero")]);
    let root = dir.path();
    let mut index = mediagit_versioning::Index::load(root).unwrap();
    index.add_entry(mediagit_versioning::IndexEntry::new(
        "art/Hero.txt".into(),
        mediagit_versioning::Oid::hash(b"hero"),
        0o100644,
        4,
        None,
    ));
    index.save(root).unwrap();

    let head = fs::read_to_string(root.join(".mediagit/refs/heads/main")).unwrap();
    let (ok, text) = run(root, &["commit", "-m", "x"]);
    assert!(!ok, "commit must refuse: {text}");
    for want in [
        "art/Hero.txt",
        "art/hero.txt",
        "Windows and macOS",
        "Rename",
    ] {
        assert!(text.contains(want), "missing {want:?} in: {text}");
    }
    assert_eq!(
        fs::read_to_string(root.join(".mediagit/refs/heads/main")).unwrap(),
        head,
        "HEAD moved on a refused commit"
    );
}

/// fsck must not call a version PERFECT that checkout will refuse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fsck_reports_two_names_that_differ_only_by_case() {
    let dir = repo_with(&[("art/hero.txt", "hero")]);
    let root = dir.path();
    plant_second_name(root, "art/hero.txt", "art/Hero.txt").await;

    // A pair that already exists is a warning (exit 0), not an error.
    let (ok, text) = run(root, &["fsck"]);
    assert!(ok, "an existing pair is a warning, not a failure:\n{text}");
    assert!(
        text.contains("art/Hero.txt") && text.contains("art/hero.txt"),
        "fsck must name both paths:\n{text}"
    );
    assert!(!text.contains("PERFECT"), "{text}");
}

/// Sibling: switching between a version with `Hero.txt` and one with
/// `hero.txt` must end with the right casing and keep the content.
#[test]
fn branch_switch_between_casings_leaves_the_right_name_on_disk() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    must(root, &["branch", "create", "other"]);
    must(root, &["branch", "switch", "other"]);
    fs::rename(root.join("art/Hero.txt"), root.join("art/hero.txt")).unwrap();
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "rename"]);

    must(root, &["branch", "switch", "main"]);
    assert_eq!(disk_names(&root.join("art")), vec!["Hero.txt"]);
    must(root, &["branch", "switch", "other"]);
    assert_eq!(disk_names(&root.join("art")), vec!["hero.txt"]);
    assert_eq!(
        fs::read_to_string(root.join("art/hero.txt")).unwrap(),
        "hero"
    );
}

/// Sibling: `reset --hard` to a commit that spells the file differently, and a
/// switch across a folder that changed case.
#[test]
fn reset_hard_and_folder_switch_across_casings_keep_the_target_spelling() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    let first = must(root, &["log", "--oneline"]);
    let first = first.split_whitespace().next().unwrap().to_string();
    fs::rename(root.join("art/Hero.txt"), root.join("art/hero.txt")).unwrap();
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "rename"]);
    must(root, &["reset", "--hard", &first]);
    assert_eq!(disk_names(&root.join("art")), vec!["Hero.txt"]);

    must(root, &["branch", "create", "folders"]);
    must(root, &["branch", "switch", "folders"]);
    fs::rename(root.join("art"), root.join("tmp-art")).unwrap();
    fs::rename(root.join("tmp-art"), root.join("ART")).unwrap();
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "folder"]);
    must(root, &["branch", "switch", "main"]);
    assert!(
        disk_names(root).contains(&"art".to_string()),
        "{:?}",
        disk_names(root)
    );
    must(root, &["branch", "switch", "folders"]);
    assert!(
        disk_names(root).contains(&"ART".to_string()),
        "{:?}",
        disk_names(root)
    );
}

fn snapshot(root: &Path, names: &[&str]) -> Vec<String> {
    names.iter().map(|n| read(root, n)).collect()
}

/// A checkout that fails validation must not touch the working tree: every
/// tracked file stays byte-identical. Any filesystem.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_checkout_leaves_every_working_file_untouched() {
    let dir = repo_with(&[("a.txt", "alpha"), ("b.txt", "beta")]);
    let root = dir.path();
    // Target drops b.txt and holds an unsafe path.
    let target = plant(
        root,
        |t| {
            let oid = t.get_entry("a.txt").unwrap().oid;
            t.entries.remove("b.txt");
            t.add_entry(TreeEntry::new(BAD.to_string(), FileMode::Regular, oid));
        },
        false,
    )
    .await;
    let (ok, text) = run(root, &["reset", "--hard", &target]);
    assert!(!ok, "reset to an unsafe tree must fail: {text}");
    assert_eq!(
        snapshot(root, &["a.txt", "b.txt"]),
        vec!["alpha", "beta"],
        "{text}"
    );
}

/// A checkout that fails while writing (object missing) must not have deleted
/// unrelated tracked files first. Any filesystem.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkout_failing_in_the_write_phase_keeps_unrelated_files() {
    let dir = repo_with(&[("a.txt", "alpha"), ("b.txt", "beta")]);
    let root = dir.path();
    let target = plant(
        root,
        |t| {
            t.entries.remove("b.txt");
            t.add_entry(TreeEntry::new(
                "n.txt".to_string(),
                FileMode::Regular,
                mediagit_versioning::Oid::hash(b"never stored"),
            ));
        },
        false,
    )
    .await;
    let (ok, text) = run(root, &["reset", "--hard", &target]);
    assert!(!ok, "reset with a missing object must fail: {text}");
    assert_eq!(
        snapshot(root, &["a.txt", "b.txt"]),
        vec!["alpha", "beta"],
        "{text}"
    );
}

/// Validation runs before the case-twin step too: a refused checkout must not
/// rename or remove the old spelling. Case-insensitive filesystems only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_checkout_does_not_touch_a_case_twin() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    let target = plant(
        root,
        |t| {
            let e = t.entries.remove("art/Hero.txt").unwrap();
            t.add_entry(TreeEntry::new(
                "art/hero.txt".into(),
                FileMode::Regular,
                e.oid,
            ));
            t.add_entry(TreeEntry::new(BAD.to_string(), FileMode::Regular, e.oid));
        },
        false,
    )
    .await;
    let (ok, text) = run(root, &["reset", "--hard", &target]);
    assert!(!ok, "{text}");
    assert_eq!(disk_names(&root.join("art")), vec!["Hero.txt"], "{text}");
    assert_eq!(read(root, "art/Hero.txt"), "hero");
}

/// A failing write after a case-only rename must still leave a file at the
/// path (the old spelling is renamed, not deleted first). Case-insensitive only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_write_after_case_rename_leaves_a_file_at_the_path() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    let target = plant(
        root,
        |t| {
            let e = t.entries.remove("art/Hero.txt").unwrap();
            // hero.txt gets new content that is not in the store.
            let _ = e;
            t.add_entry(TreeEntry::new(
                "art/hero.txt".into(),
                FileMode::Regular,
                mediagit_versioning::Oid::hash(b"never stored"),
            ));
        },
        false,
    )
    .await;
    let (ok, text) = run(root, &["reset", "--hard", &target]);
    assert!(!ok, "{text}");
    assert_eq!(disk_names(&root.join("art")).len(), 1, "{text}");
    assert_eq!(
        read(root, "art/hero.txt"),
        "hero",
        "content survives: {text}"
    );
}

/// Same guarantee through the differential path (branch switch): a target
/// holding a colliding pair (fails validation) must not rename the case twin first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_branch_switch_does_not_touch_a_case_twin() {
    let dir = repo_with(&[("art/Hero.txt", "hero"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    let target = plant(
        root,
        |t| {
            let e = t.entries.remove("art/Hero.txt").unwrap();
            t.add_entry(TreeEntry::new(
                "art/hero.txt".into(),
                FileMode::Regular,
                e.oid,
            ));
            t.add_entry(TreeEntry::new(
                "art/HERO.txt".into(),
                FileMode::Regular,
                e.oid,
            ));
        },
        false,
    )
    .await;
    let refdb = RefDatabase::new(root.join(".mediagit"));
    refdb
        .write(&Ref::new_direct(
            "refs/heads/planted".to_string(),
            mediagit_versioning::Oid::from_hex(&target).unwrap(),
        ))
        .await
        .unwrap();
    let (ok, text) = run(root, &["branch", "switch", "planted"]);
    assert!(!ok, "{text}");
    assert_eq!(disk_names(&root.join("art")), vec!["Hero.txt"], "{text}");
    assert_eq!(read(root, "art/Hero.txt"), "hero");
}

/// A failing write after a case-only FOLDER rename must still leave the file
/// readable (the old file is not removed first). Case-insensitive only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_write_after_folder_case_rename_keeps_the_file() {
    let dir = repo_with(&[("art/x.txt", "x"), ("keep.txt", "keep")]);
    let root = dir.path();
    if !case_insensitive(root) {
        return;
    }
    let target = plant(
        root,
        |t| {
            t.entries.remove("art/x.txt");
            t.add_entry(TreeEntry::new(
                "Art/x.txt".into(),
                FileMode::Regular,
                mediagit_versioning::Oid::hash(b"never stored"),
            ));
        },
        false,
    )
    .await;
    let (ok, text) = run(root, &["reset", "--hard", &target]);
    assert!(!ok, "{text}");
    assert_eq!(read(root, "art/x.txt"), "x", "file survives: {text}");
}

/// An unreadable folder stops `add` with an error naming it, and stages no
/// deletion for its files. Windows: the folder's list permission is denied.
#[cfg(windows)]
#[test]
fn unreadable_folder_stops_add_and_stages_no_deletion() {
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::process::Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .output();
        }
    }
    let dir = repo_with(&[("locked/a.txt", "a"), ("keep.txt", "keep")]);
    let root = dir.path();
    let locked = root.join("locked");
    // Ignored, so the working-tree walk skips the folder and only the
    // deletion scan (the code under test) has to read it.
    fs::write(
        root.join(".mediagitignore"),
        "locked/
",
    )
    .unwrap();
    let _restore = Restore(locked.clone());
    let out = std::process::Command::new("icacls")
        .arg(&locked)
        .args(["/deny", "*S-1-1-0:(RD)"])
        .output()
        .unwrap();
    assert!(out.status.success(), "icacls: {:?}", out);
    if fs::read_dir(&locked).is_ok() {
        return; // the host ignores the deny (for example an elevated user)
    }
    let (ok, text) = run(root, &["add", "--all"]);
    assert!(!ok, "add must stop: {text}");
    assert!(
        text.contains("Cannot read the folder") && text.contains("locked"),
        "must name the folder: {text}"
    );
    // --ignore-removal never needs the answer, so it must not fail.
    let (ok, text) = run(root, &["add", "--all", "--ignore-removal"]);
    assert!(ok, "--ignore-removal must not read the folder: {text}");
    drop(_restore);
    let (_, status) = run(root, &["status", "--porcelain"]);
    assert!(
        !status.lines().any(|l| l.starts_with('D')),
        "no deletion may be staged:\n{status}"
    );
}
