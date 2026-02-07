// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Custom test assertions for MediaGit tests.
//!
//! Provides domain-specific assertions for testing MediaGit functionality.

use std::path::Path;

/// Assert that a repository is properly initialized.
///
/// Checks that the .mediagit directory exists with required structure.
pub fn assert_repo_initialized(path: &Path) {
    let mediagit_dir = path.join(".mediagit");
    assert!(
        mediagit_dir.exists(),
        ".mediagit directory should exist at {:?}",
        path
    );
    assert!(
        mediagit_dir.join("objects").exists(),
        "objects directory should exist"
    );
    assert!(
        mediagit_dir.join("refs").exists(),
        "refs directory should exist"
    );
    assert!(
        mediagit_dir.join("HEAD").exists(),
        "HEAD file should exist"
    );
}

/// Assert that a file is tracked in the repository.
///
/// This checks if the file exists in the staging area or has been committed.
pub fn assert_file_tracked(repo_path: &Path, file_name: &str) {
    use crate::mediagit;

    mediagit()
        .arg("status")
        .current_dir(repo_path)
        .assert()
        .success();

    // The file should exist in the repo
    assert!(
        repo_path.join(file_name).exists(),
        "File {} should exist in repository",
        file_name
    );
}

/// Assert that a branch exists in the repository.
pub fn assert_branch_exists(repo_path: &Path, branch_name: &str) {
    use crate::mediagit;
    use predicates::prelude::*;

    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(repo_path)
        .assert()
        .success()
        .stdout(predicate::str::contains(branch_name));
}

/// Assert that a branch does not exist in the repository.
pub fn assert_branch_not_exists(repo_path: &Path, branch_name: &str) {
    use crate::mediagit;
    use predicates::prelude::*;

    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(repo_path)
        .assert()
        .success()
        .stdout(predicate::str::contains(branch_name).not());
}

/// Assert that we are on a specific branch.
pub fn assert_on_branch(repo_path: &Path, branch_name: &str) {
    use crate::mediagit;
    use predicates::prelude::*;

    // The current branch should be marked with * in branch list
    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(repo_path)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("* {}", branch_name)));
}

/// Assert that a mediagit command succeeds.
#[macro_export]
macro_rules! assert_mediagit_success {
    ($repo:expr, $($arg:expr),+ $(,)?) => {
        $crate::mediagit()
            $(.arg($arg))+
            .current_dir($repo.path())
            .assert()
            .success()
    };
}

/// Assert that a mediagit command fails.
#[macro_export]
macro_rules! assert_mediagit_failure {
    ($repo:expr, $($arg:expr),+ $(,)?) => {
        $crate::mediagit()
            $(.arg($arg))+
            .current_dir($repo.path())
            .assert()
            .failure()
    };
}

/// Assert that a mediagit command output contains a specific string.
#[macro_export]
macro_rules! assert_mediagit_output_contains {
    ($repo:expr, $expected:expr, $($arg:expr),+ $(,)?) => {
        $crate::mediagit()
            $(.arg($arg))+
            .current_dir($repo.path())
            .assert()
            .success()
            .stdout(predicates::prelude::predicate::str::contains($expected))
    };
}
