// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::{create_storage_backend, find_repo_root};
use super::status::StatusCmd;
use crate::ignore_rules::IgnoreMatcher;
use anyhow::Result;
use clap::Parser;
use mediagit_versioning::{Index, ObjectDatabase, Oid, RefDatabase, SparseFilter};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Working-folder status of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
#[serde(rename_all = "snake_case")]
pub enum LsFileStatus {
    Unchanged,
    Modified,
    New,
    Deleted,
    Ignored,
}

/// One file in the working folder.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct LsFilesEntry {
    /// Repo-relative, forward slashes.
    pub path: String,
    pub status: LsFileStatus,
    /// The index holds a change for this path (staged add, edit or delete).
    pub staged: bool,
    /// Bytes on disk; absent for deleted files.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub size: Option<u64>,
}

/// `--machine` data for `ls-files`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct LsFilesReport {
    pub files: Vec<LsFilesEntry>,
}

/// List every file in the working folder with its status
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Every file, with its status
    mediagit ls-files

    # Only one folder, ignored files included
    mediagit ls-files --ignored assets/

SEE ALSO:
    mediagit-status(1)")]
pub struct LsFilesCmd {
    /// Only list files under these repo-relative paths (default: all)
    pub paths: Vec<String>,

    /// Also list files matched by .mediagitignore
    #[arg(long)]
    pub ignored: bool,
}

/// Repo-relative prefix filter: exact file or anything beneath a folder.
fn wanted(filters: &[String], path: &str) -> bool {
    filters.is_empty()
        || filters.iter().any(|f| {
            f.is_empty()
                || path == f
                || path
                    .strip_prefix(f.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
}

fn normalize_filter(f: &str) -> String {
    let f = f.replace('\\', "/");
    let f = f.trim_start_matches("./").trim_end_matches('/');
    if f == "." {
        String::new()
    } else {
        f.to_owned()
    }
}

impl LsFilesCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        let repo_root = dunce::canonicalize(find_repo_root()?)
            .unwrap_or_else(|_| find_repo_root().expect("repo root"));
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(repo_root.join(".mediagit"));
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let index = Index::load(&repo_root)?;

        // Same walk and ignore rules as `status`.
        let mut ignored_files: HashSet<PathBuf> = HashSet::new();
        let matcher = IgnoreMatcher::new(&repo_root).ok();
        let walker = StatusCmd {
            tracked: false,
            untracked: false,
            ignored: false,
            short: false,
            porcelain: false,
            branch: false,
            quiet: true,
            verbose: false,
            json: false,
        };
        let working_files =
            walker.scan_working_directory(&repo_root, &matcher, &mut ignored_files)?;

        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        if let Ok(head_oid) = refdb.resolve("HEAD").await
            && let Ok(data) = odb.read(&head_oid).await
            && let Ok(commit) =
                mediagit_versioning::format::deserialize::<mediagit_versioning::Commit>(&data)
            && let Ok(tree_data) = odb.read(&commit.tree).await
            && let Ok(tree) =
                mediagit_versioning::format::deserialize::<mediagit_versioning::Tree>(&tree_data)
        {
            for e in tree.iter() {
                head_files.insert(PathBuf::from(&e.name), e.oid);
            }
        }

        // An index entry identical to HEAD is not a staged change.
        let staged: HashSet<PathBuf> = index
            .entries()
            .filter(|e| head_files.get(&e.path) != Some(&e.oid))
            .map(|e| e.path.clone())
            .collect();

        let sparse = SparseFilter::load(&repo_root).unwrap_or_else(|_| SparseFilter::disabled());
        let filters: Vec<String> = self.paths.iter().map(|p| normalize_filter(p)).collect();
        let key = |p: &Path| p.to_string_lossy().replace('\\', "/");

        // Tracked, unstaged files: modified iff the content hash differs from HEAD.
        let mut stat_cache = mediagit_versioning::StatCache::load(&repo_root);
        type Hashed = (PathBuf, bool, Option<(std::fs::Metadata, Oid)>);
        let hashed: Vec<Hashed> = head_files
            .par_iter()
            .filter(|(p, _)| {
                working_files.contains(*p) && !staged.contains(*p) && wanted(&filters, &key(p))
            })
            .filter_map(|(path, head_oid)| {
                let full = repo_root.join(path);
                let meta = std::fs::symlink_metadata(&full).ok()?;
                if let Some(oid) = stat_cache.lookup(&path.to_string_lossy(), &meta) {
                    return Some((path.clone(), oid != *head_oid, None));
                }
                let oid = if meta.len() >= super::utils::STREAMING_THRESHOLD {
                    Oid::from_file(&full).ok()?
                } else {
                    Oid::hash(&std::fs::read(&full).ok()?)
                };
                Some((path.clone(), oid != *head_oid, Some((meta, oid))))
            })
            .collect();
        let mut modified: HashSet<PathBuf> = HashSet::new();
        for (path, is_modified, record) in hashed {
            if let Some((meta, oid)) = record {
                stat_cache.record(&path.to_string_lossy(), &meta, oid);
            }
            if is_modified {
                modified.insert(path);
            }
        }
        if let Err(e) = stat_cache.save() {
            tracing::debug!("stat cache not saved: {e}");
        }

        let size = |p: &Path| std::fs::metadata(repo_root.join(p)).ok().map(|m| m.len());
        let mut files: Vec<LsFilesEntry> = Vec::new();
        for path in &working_files {
            let s = key(path);
            if !wanted(&filters, &s) {
                continue;
            }
            let is_staged = staged.contains(path);
            let status = if !head_files.contains_key(path) {
                LsFileStatus::New
            } else if is_staged || modified.contains(path) {
                LsFileStatus::Modified
            } else {
                LsFileStatus::Unchanged
            };
            files.push(LsFilesEntry {
                path: s,
                status,
                staged: is_staged,
                size: size(path),
            });
        }
        for path in head_files.keys() {
            let s = key(path);
            if !working_files.contains(path) && sparse.is_included(path) && wanted(&filters, &s) {
                files.push(LsFilesEntry {
                    path: s,
                    status: LsFileStatus::Deleted,
                    staged: index.is_deleted(path),
                    size: None,
                });
            }
        }
        if self.ignored {
            for path in &ignored_files {
                let s = key(path);
                if wanted(&filters, &s) {
                    files.push(LsFilesEntry {
                        path: s,
                        status: LsFileStatus::Ignored,
                        staged: false,
                        size: size(path),
                    });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));

        let report = LsFilesReport { files };
        if output::machine_mode() {
            return output::emit_ok(&report);
        }
        for f in &report.files {
            let tag = match f.status {
                LsFileStatus::Unchanged => ' ',
                LsFileStatus::Modified => 'M',
                LsFileStatus::New => 'N',
                LsFileStatus::Deleted => 'D',
                LsFileStatus::Ignored => '!',
            };
            println!("{tag}{} {}", if f.staged { '*' } else { ' ' }, f.path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_match_files_and_folders_not_siblings() {
        let f = vec![normalize_filter("./assets/"), normalize_filter("a.txt")];
        assert!(wanted(&f, "assets/x/y.psd"));
        assert!(wanted(&f, "a.txt"));
        assert!(!wanted(&f, "assets2/x"));
        assert!(!wanted(&f, "b.txt"));
        assert!(wanted(&[normalize_filter(".")], "anything"));
        assert!(wanted(&[], "anything"));
    }
}
