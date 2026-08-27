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

//! CLI Completions Tests
//!
//! Tests for `mediagit completions` command to ensure completion scripts
//! are properly generated for shell integration.

use assert_cmd::Command;

#[allow(deprecated)]
fn mediagit() -> Command {
    {
        // `commit` refuses an unconfigured identity (UX-6) instead of
        // authoring as `Unknown <unknown@localhost>`, so tests declare one
        // the way a real user would.
        let mut c = Command::cargo_bin("mediagit").unwrap();
        c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
            .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
        c
    }
}

#[test]
fn test_completions_bash_outputs_to_stdout() {
    let output = mediagit().arg("completions").arg("bash").output().unwrap();

    assert!(output.status.success(), "completions bash should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify non-empty stdout
    assert!(
        !stdout.is_empty(),
        "completions bash should write non-empty output to stdout"
    );

    // Verify it looks like a bash completion script
    assert!(
        stdout.contains("complete") || stdout.contains("bash"),
        "completions bash should contain shell-related content"
    );
}

#[test]
fn test_completions_zsh_outputs_to_stdout() {
    let output = mediagit().arg("completions").arg("zsh").output().unwrap();

    assert!(output.status.success(), "completions zsh should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify non-empty stdout
    assert!(
        !stdout.is_empty(),
        "completions zsh should write non-empty output to stdout"
    );

    // Verify it looks like a zsh completion script
    assert!(
        stdout.contains("compctl") || stdout.contains("#"),
        "completions zsh should contain shell-related content"
    );
}

#[test]
fn test_completions_fish_outputs_to_stdout() {
    let output = mediagit().arg("completions").arg("fish").output().unwrap();

    assert!(output.status.success(), "completions fish should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify non-empty stdout
    assert!(
        !stdout.is_empty(),
        "completions fish should write non-empty output to stdout"
    );

    // Verify it looks like a fish completion script
    assert!(
        stdout.contains("complete") || stdout.contains("function"),
        "completions fish should contain shell-related content"
    );
}
