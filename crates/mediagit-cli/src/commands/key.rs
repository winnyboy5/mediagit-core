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

//! Manage this repository's at-rest encryption key (DC-7/D2).
//!
//! Thin CLI over [`crate::encryption`], which owns the on-disk format and the
//! master-key precedence. Nothing here prints, logs, or returns key material —
//! not the repo key, not the master key, not a prefix of either.

use crate::encryption;
use crate::output;
use crate::repo::find_repo_root;
use anyhow::Result;
use clap::{Parser, Subcommand};

/// Manage at-rest encryption for this repository
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Turn on at-rest encryption for this repository
    mediagit key init

    # Show whether this repository is encrypted, and how it unlocks
    mediagit key status

    # Use a key file instead of the OS keychain
    MEDIAGIT_ENCRYPTION_KEYFILE=/media/usb/mediagit.key mediagit key init

    # Master key lost — get back in with the code printed at `key init`
    mediagit key recover

NOTES:
    Objects written after `key init` are sealed; objects written before it stay
    readable. There is no `key rotate` and no way to remove a key: both would
    have to rewrite every object in the repository.

    The master key is not stored in the repository. `key init` prints a one-time
    recovery code which is the only other way in; if both are lost, the objects
    cannot be recovered by MediaGit or by anyone else.")]
pub struct EncryptionKeyCmd {
    #[command(subcommand)]
    pub subcommand: KeySubcommand,
}

#[derive(Subcommand, Debug)]
pub enum KeySubcommand {
    /// Generate and store this repository's encryption key
    Init,

    /// Show whether this repository is encrypted and how it unlocks
    Status,

    /// Unlock with the recovery code and re-wrap under this machine's master key
    Recover(RecoverOpts),
}

/// Unlock with the recovery code and re-wrap under this machine's master key
#[derive(Parser, Debug)]
pub struct RecoverOpts {
    /// The recovery code. Prompted for if omitted — which is the better way to
    /// supply it, since an argument lands in shell history.
    #[arg(value_name = "CODE")]
    pub code: Option<String>,
}

impl EncryptionKeyCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        match &self.subcommand {
            KeySubcommand::Init => {
                let outcome = encryption::init_repo_key(&repo_root)?;
                output::success("At-rest encryption enabled for this repository");
                println!("  Master key: {}", outcome.source.describe());
                println!(
                    "  Wrapped key: {}",
                    encryption::key_file_path(&repo_root).display()
                );
                println!();

                // The one deliberate display of key material in the whole
                // feature. Written to stdout and nowhere else: not tracing, not
                // the key file, not the clipboard. There is no second chance to
                // show it, so it is framed to look like something to copy.
                println!("  ┌─ RECOVERY CODE ─ write this down now ─────────────┐");
                println!("     {}", outcome.recovery_code.as_str());
                println!("  └───────────────────────────────────────────────────┘");
                println!(
                    "  This is the only time it is shown, and the only way back in if the\n  \
                     master key is lost. It is not stored anywhere."
                );
                println!();

                output::warning(
                    "Objects written from now on are encrypted. Lose both the master key \
                     and the recovery code and they cannot be recovered.",
                );
                if matches!(outcome.source, encryption::MasterSource::Passphrase) {
                    println!(
                        "  You will be asked for this passphrase by every command that reads \
                         or writes objects."
                    );
                }
                Ok(())
            }
            KeySubcommand::Recover(opts) => {
                let code = match opts.code.clone() {
                    Some(code) => code,
                    None => dialoguer::Password::new()
                        .with_prompt("Recovery code")
                        .interact()?,
                };
                let source = encryption::recover_repo_key(&repo_root, &code)?;
                output::success("Repository unlocked and re-wrapped");
                println!("  Master key: {}", source.describe());
                println!("  Your recovery code is unchanged — keep it.");
                Ok(())
            }
            KeySubcommand::Status => {
                if !encryption::has_key(&repo_root) {
                    println!("At-rest encryption: off");
                    println!("  Run `mediagit key init` to enable it.");
                    return Ok(());
                }
                println!("At-rest encryption: on");
                println!(
                    "  Wrapped key: {}",
                    encryption::key_file_path(&repo_root).display()
                );
                match encryption::available_master_source(&repo_root)? {
                    Some(source) => println!("  Unlocks with: {}", source.describe()),
                    None => output::warning(
                        "No master key is available on this machine — objects in this \
                         repository cannot currently be read.",
                    ),
                }
                if encryption::has_recovery_slot(&repo_root)? {
                    println!("  Recovery slot: present (`mediagit key recover`)");
                } else {
                    println!("  Recovery slot: none — the master key is the only way in");
                }
                Ok(())
            }
        }
    }
}
