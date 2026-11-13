// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-008 fix round 1: a repository that already holds an absolute or `..`
//! path (left by an older release) must have a way out, and a refused
//! operation must leave refs and merge state untouched.
//!
//! The bad entries are planted through the library API, bypassing the
//! write-side guard on purpose.

use assert_cmd::Command;
use mediagit_versioning::{
    Commit, FileMode, ObjectType, Ref, RefDatabase, Signature, Tree, TreeEntry,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const BAD: &str = "//?/C:/Users/someone/work/a.txt";

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    cmd
}

/// Run and return (success, stdout + stderr).
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

/// Repo with a.txt + b.txt committed on main, and a branch `other` at that
/// commit.
fn clean_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    must(root, &["init", "-q"]);
    fs::write(root.join("a.txt"), "alpha").unwrap();
    fs::write(root.join("b.txt"), "beta").unwrap();
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "first"]);
    must(root, &["branch", "create", "other"]);
    dir
}

fn head_ref_file(root: &Path) -> String {
    // HEAD names the branch; the branch file holds the oid.
    let head = fs::read_to_string(root.join(".mediagit/HEAD")).unwrap();
    let branch = head.trim().trim_start_matches("ref: ").to_string();
    fs::read_to_string(root.join(".mediagit").join(&branch))
        .unwrap()
        .trim()
        .to_string()
}

/// Rewrite HEAD's commit so its tree names `a.txt` as `BAD`.
async fn poison_head(root: &Path) {
    let oid = poison_commit(root).await;
    let refdb = RefDatabase::new(root.join(".mediagit"));
    let head_ref = refdb.read("HEAD").await.unwrap();
    let branch = head_ref.target.expect("HEAD names a branch");
    refdb.write(&Ref::new_direct(branch, oid)).await.unwrap();
}

/// A child of HEAD whose tree holds `BAD`; no ref is moved.
async fn poison_commit(root: &Path) -> mediagit_versioning::Oid {
    let storage = mediagit_cli::repo::create_storage_backend(root)
        .await
        .unwrap();
    let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 100);
    let refdb = RefDatabase::new(root.join(".mediagit"));
    let head = refdb.resolve("HEAD").await.unwrap();
    let commit = Commit::deserialize(&odb.read(&head).await.unwrap()).unwrap();
    let tree = Tree::deserialize(&odb.read(&commit.tree).await.unwrap()).unwrap();
    let a = tree.get_entry("a.txt").unwrap().clone();
    let mut bad = Tree::new();
    for e in tree.iter().filter(|e| e.name != "a.txt") {
        bad.add_entry(e.clone());
    }
    bad.add_entry(TreeEntry::new(BAD.to_string(), FileMode::Regular, a.oid));
    // Bypass `Tree::serialize`, which refuses this name.
    let tree_oid = odb
        .write(
            ObjectType::Tree,
            &mediagit_versioning::format::serialize(&bad).unwrap(),
        )
        .await
        .unwrap();
    let sig = Signature::now("t".into(), "t@e".into());
    let poisoned = Commit::with_parents(tree_oid, vec![head], sig.clone(), sig, "poisoned".into());
    poisoned.write(&odb).await.unwrap()
}

/// Plant `BAD` in the staging area in place of `c.txt`.
fn poison_index(root: &Path) {
    fs::write(root.join("c.txt"), "gamma").unwrap();
    must(root, &["add", "c.txt"]);
    let path = root.join(".mediagit/index");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("c.txt"));
    fs::write(&path, text.replace("c.txt", BAD)).unwrap();
}

/// Every name in HEAD's tree, via ls-files.
fn head_tree_is_clean(root: &Path) {
    let out = must(root, &["ls-files"]);
    assert!(!out.contains("//?/"), "tree still holds a bad path:\n{out}");
    assert!(out.contains("a.txt") && out.contains("b.txt"), "{out}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_head_entry_add_all_is_the_way_out() {
    let dir = clean_repo();
    let root = dir.path();
    poison_head(root).await;

    // Refused, naming the path and the way out; nothing moves.
    let before = head_ref_file(root);
    let (ok, text) = run(root, &["commit", "--allow-empty", "-m", "x"]);
    assert!(!ok, "commit on a bad tree must fail: {text}");
    assert!(text.contains(BAD), "must name the path: {text}");
    assert!(
        text.contains("mediagit add --all"),
        "must name the way out: {text}"
    );
    assert_eq!(
        head_ref_file(root),
        before,
        "HEAD moved on a refused commit"
    );

    // Way out: add --all drops the bad entry and re-adds a.txt.
    let out = must(root, &["add", "--all"]);
    assert!(
        out.contains("unsafe path"),
        "add must say what it did: {out}"
    );
    must(root, &["commit", "-m", "repair"]);
    head_tree_is_clean(root);

    // History keeps the old commit; fsck still reports that one tree and
    // names the path. The repaired HEAD adds nothing new.
    let (_, fsck) = run(root, &["fsck"]);
    assert_eq!(fsck.matches(BAD).count(), 1, "fsck output:\n{fsck}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_head_entry_refuses_merge_before_any_state_is_written() {
    let dir = clean_repo();
    let root = dir.path();
    poison_head(root).await;

    let before = head_ref_file(root);
    let (ok, text) = run(root, &["merge", "other"]);
    assert!(!ok, "merge on a bad tree must fail: {text}");
    assert!(text.contains(BAD), "must name the path: {text}");
    assert_eq!(head_ref_file(root), before, "HEAD moved on a refused merge");
    for state in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "ORIG_HEAD"] {
        assert!(
            !root.join(".mediagit").join(state).exists(),
            "{state} left behind"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_head_entry_refuses_rebase_cherrypick_revert_and_stash() {
    let dir = clean_repo();
    let root = dir.path();
    poison_head(root).await;
    let before = head_ref_file(root);
    for args in [
        vec!["rebase", "other"],
        vec!["cherry-pick", "other"],
        vec!["revert", "HEAD"],
        vec!["stash", "push"],
    ] {
        let (ok, text) = run(root, &args);
        assert!(!ok, "{args:?} must fail on a bad tree: {text}");
        assert!(text.contains(BAD), "{args:?} must name the path: {text}");
        assert_eq!(head_ref_file(root), before, "{args:?} moved HEAD");
    }
    assert!(!root.join(".mediagit/REBASE_STATE").exists());
}

#[test]
fn bad_index_entry_add_all_is_the_way_out() {
    let dir = clean_repo();
    let root = dir.path();
    poison_index(root);

    let before = head_ref_file(root);
    let (ok, text) = run(root, &["commit", "-m", "x"]);
    assert!(!ok, "commit with a bad staged path must fail: {text}");
    assert!(text.contains(BAD), "must name the path: {text}");
    assert!(
        text.contains("mediagit reset --"),
        "must name the way out: {text}"
    );
    assert_eq!(head_ref_file(root), before);

    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "repair"]);
    let out = must(root, &["ls-files"]);
    assert!(!out.contains("//?/"), "{out}");
    assert!(
        out.contains("c.txt"),
        "c.txt re-added under its own name: {out}"
    );
    // No version ever held the bad path, so fsck is clean.
    let (ok, fsck) = run(root, &["fsck"]);
    assert!(ok && !fsck.contains(BAD), "fsck:\n{fsck}");
}

#[test]
fn bad_index_entry_reset_path_is_a_way_out() {
    let dir = clean_repo();
    let root = dir.path();
    poison_index(root);

    must(root, &["reset", "--", BAD]);
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "repair"]);
    let out = must(root, &["ls-files"]);
    assert!(!out.contains("//?/") && out.contains("c.txt"), "{out}");
}

#[test]
fn bad_index_entry_refuses_merge_before_any_state_is_written() {
    let dir = clean_repo();
    let root = dir.path();
    poison_index(root);

    let before = head_ref_file(root);
    let (ok, text) = run(root, &["merge", "other"]);
    assert!(!ok, "{text}");
    assert!(text.contains(BAD), "{text}");
    assert_eq!(head_ref_file(root), before);
    assert!(!root.join(".mediagit/MERGE_HEAD").exists());
}

/// A symlink that leaves the repository: add names the link, says how to
/// silence it, exits non-zero, and still stages the rest.
#[test]
fn symlink_outside_repository_is_named_and_refused() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    must(root, &["init", "-q"]);
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("big.bin"), "asset").unwrap();
    fs::write(root.join("ok.txt"), "fine").unwrap();
    let link = root.join("assets_link");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(outside.path().join("big.bin"), &link);
    // Symlinks need a privilege on Windows; a junction does not, and the add
    // walk meets its files through the same "canonical path leaves the repo".
    #[cfg(windows)]
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .map(|o| o.status.success())
        .map_err(|e| e.to_string())
        .and_then(|ok| {
            if ok {
                Ok(())
            } else {
                Err("mklink failed".to_string())
            }
        });
    if made.is_err() {
        eprintln!("cannot create a link here; skipping");
        return;
    }

    let (ok, text) = run(root, &["add", "--all"]);
    assert!(!ok, "add must exit non-zero: {text}");
    assert!(text.contains("assets_link"), "must name the link: {text}");
    assert!(
        !text.contains(&outside.path().to_string_lossy().to_string()),
        "must not name the target: {text}"
    );
    assert!(
        text.contains(".mediagitignore"),
        "must suggest an ignore rule: {text}"
    );
    let (_, st) = run(root, &["status", "--porcelain"]);
    assert!(st.contains("ok.txt"), "{st}");
    assert!(!st.contains("//?/"), "{st}");
}

/// Names in HEAD's tree.
async fn head_tree_names(root: &Path) -> Vec<String> {
    let storage = mediagit_cli::repo::create_storage_backend(root)
        .await
        .unwrap();
    let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 100);
    let refdb = RefDatabase::new(root.join(".mediagit"));
    let head = refdb.resolve("HEAD").await.unwrap();
    let commit = Commit::deserialize(&odb.read(&head).await.unwrap()).unwrap();
    let tree = Tree::deserialize(&odb.read(&commit.tree).await.unwrap()).unwrap();
    tree.iter().map(|e| e.name.clone()).collect()
}

/// Cherry-picking `A P` where P holds a bad path: A must not be committed
/// before P is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cherry_pick_checks_every_listed_commit_before_applying_any() {
    let dir = clean_repo();
    let root = dir.path();
    fs::write(root.join("c.txt"), "gamma").unwrap();
    must(root, &["add", "c.txt"]);
    must(root, &["commit", "-m", "A"]);
    let a = head_ref_file(root);
    let p = poison_commit(root).await.to_hex();
    must(root, &["branch", "switch", "other"]);

    let before = head_ref_file(root);
    let (ok, text) = run(root, &["cherry-pick", &a, &p]);
    assert!(!ok, "{text}");
    assert!(text.contains(BAD), "{text}");
    assert_eq!(
        head_ref_file(root),
        before,
        "A was committed before P was refused"
    );
    assert!(!root.join(".mediagit/CHERRY_PICK_STATE").exists());
}

/// Bad entry whose file is gone: it is removed and the message says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_head_entry_with_missing_file_is_removed_and_named() {
    let dir = clean_repo();
    let root = dir.path();
    poison_head(root).await;
    fs::remove_file(root.join("a.txt")).unwrap();

    let out = must(root, &["add", "--all"]);
    assert!(
        out.contains(BAD) && out.contains("not in the working folder"),
        "{out}"
    );
    must(root, &["commit", "-m", "repair"]);
    let names = head_tree_names(root).await;
    assert!(names.iter().all(|n| !n.contains("//?/")), "{names:?}");
    assert!(!names.contains(&"a.txt".to_string()), "{names:?}");
    assert!(names.contains(&"b.txt".to_string()), "{names:?}");
}

/// The repair is named even under --quiet, and --machine carries the list.
#[test]
fn repair_is_named_under_quiet_and_in_machine_output() {
    let dir = clean_repo();
    let root = dir.path();
    poison_index(root);
    let (ok, text) = run(root, &["add", "--all", "--quiet"]);
    assert!(ok, "{text}");
    assert!(
        text.contains(BAD),
        "quiet add must still name the path: {text}"
    );

    let dir = clean_repo();
    let root = dir.path();
    poison_index(root);
    let (ok, text) = run(root, &["--machine", "add", "--all"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("repaired_unsafe_paths") && text.contains(BAD),
        "{text}"
    );
}

/// Sparse checkout: repairing a bad entry must not stage the removal of the
/// good entries that sparse checkout excluded (BUG-041 stays fixed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_entry_repair_leaves_sparse_excluded_entries_alone() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    must(root, &["init", "-q"]);
    fs::create_dir_all(root.join("included")).unwrap();
    fs::create_dir_all(root.join("excluded")).unwrap();
    fs::write(root.join("a.txt"), "alpha").unwrap();
    fs::write(root.join("included/keep.txt"), "k").unwrap();
    fs::write(root.join("excluded/drop.txt"), "d").unwrap();
    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "first"]);
    must(root, &["sparse-checkout", "set", "included"]);
    assert!(!root.join("excluded/drop.txt").exists());
    poison_head(root).await;

    must(root, &["add", "--all"]);
    must(root, &["commit", "-m", "repair"]);
    let names = head_tree_names(root).await;
    assert!(
        names.contains(&"excluded/drop.txt".to_string()),
        "excluded entry lost: {names:?}"
    );
    assert!(
        names.contains(&"included/keep.txt".to_string()),
        "{names:?}"
    );
    assert!(names.iter().all(|n| !n.contains("//?/")), "{names:?}");
}

/// A linked folder that leaves the repository is named once, not per file.
#[cfg(windows)]
#[test]
fn linked_folder_outside_repository_warns_once() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    must(root, &["init", "-q"]);
    let outside = TempDir::new().unwrap();
    for i in 0..5 {
        fs::write(outside.path().join(format!("f{i}.bin")), "x").unwrap();
    }
    let ok = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("assets_link"))
        .arg(outside.path())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return;
    }
    let (ok, text) = run(root, &["add", "--all"]);
    assert!(!ok, "{text}");
    assert_eq!(
        text.matches("Cannot track").count(),
        1,
        "one warning for the link, not one per file: {text}"
    );
}
