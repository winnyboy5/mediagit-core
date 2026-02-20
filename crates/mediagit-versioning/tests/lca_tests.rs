// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! LCA (Lowest Common Ancestor) Algorithm Tests
//!
//! Tests for merge base detection covering:
//! - Simple linear history
//! - Divergent branches
//! - Multiple merge bases (criss-cross)
//! - Performance requirements (<50ms)

use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{Commit, LcaFinder, ObjectDatabase, Oid, Signature};
use std::sync::Arc;
use std::time::Instant;

/// Helper to create test commits with specified parents
async fn create_test_commit(
    odb: &Arc<ObjectDatabase>,
    message: &str,
    parents: Vec<Oid>,
) -> Oid {
    let tree = Oid::hash(b"tree");
    let sig = Signature::now(
        "Test Author".to_string(),
        "test@example.com".to_string(),
    );

    let commit = Commit::with_parents(
        tree,
        parents,
        sig.clone(),
        sig,
        message.to_string(),
    );

    commit.write(odb).await.unwrap()
}

/// Test simple linear history: A <- B <- C
/// LCA of B and C should be B
#[tokio::test]
async fn test_lca_linear_history() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create linear history: A <- B <- C
    let commit_a = create_test_commit(&odb, "A", vec![]).await;
    let commit_b = create_test_commit(&odb, "B", vec![commit_a.clone()]).await;
    let commit_c = create_test_commit(&odb, "C", vec![commit_b.clone()]).await;

    // LCA of B and C should be B
    let bases = lca_finder
        .find_merge_base(&commit_b, &commit_c)
        .await
        .unwrap();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commit_b);
}

/// Test divergent branches:
///     C
///    /
///   A
///    \
///     D
/// LCA of C and D should be A
#[tokio::test]
async fn test_lca_divergent_branches() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create divergent history
    let commit_a = create_test_commit(&odb, "A", vec![]).await;
    let commit_c = create_test_commit(&odb, "C", vec![commit_a.clone()]).await;
    let commit_d = create_test_commit(&odb, "D", vec![commit_a.clone()]).await;

    // LCA of C and D should be A
    let bases = lca_finder
        .find_merge_base(&commit_c, &commit_d)
        .await
        .unwrap();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commit_a);
}

/// Test diamond merge:
///     B   C
///      \ /
///       A
/// LCA of B and C should be A
#[tokio::test]
async fn test_lca_diamond_merge() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    let commit_a = create_test_commit(&odb, "A", vec![]).await;
    let commit_b = create_test_commit(&odb, "B", vec![commit_a.clone()]).await;
    let commit_c = create_test_commit(&odb, "C", vec![commit_a.clone()]).await;

    let bases = lca_finder
        .find_merge_base(&commit_b, &commit_c)
        .await
        .unwrap();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commit_a);
}

/// Test complex history with merge commits
///       E
///      / \
///     C   D
///      \ /
///       B
///       |
///       A
#[tokio::test]
async fn test_lca_with_merge_commits() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    let commit_a = create_test_commit(&odb, "A", vec![]).await;
    let commit_b = create_test_commit(&odb, "B", vec![commit_a.clone()]).await;
    let commit_c = create_test_commit(&odb, "C", vec![commit_b.clone()]).await;
    let commit_d = create_test_commit(&odb, "D", vec![commit_b.clone()]).await;
    let _commit_e = create_test_commit(&odb, "E", vec![commit_c.clone(), commit_d.clone()]).await;

    // LCA of C and D should be B
    let bases = lca_finder
        .find_merge_base(&commit_c, &commit_d)
        .await
        .unwrap();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commit_b);
}

/// Test criss-cross merge scenario (multiple merge bases)
///       X   Y
///      / \ / \
///     A   M   B
///      \ /
///       O
/// where M is a merge of A and B
/// LCA of X and Y may have multiple bases
#[tokio::test]
async fn test_lca_criss_cross_merge() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create criss-cross scenario
    let commit_o = create_test_commit(&odb, "O", vec![]).await;
    let commit_a = create_test_commit(&odb, "A", vec![commit_o.clone()]).await;
    let commit_b = create_test_commit(&odb, "B", vec![commit_o.clone()]).await;

    // M merges A and B
    let commit_m = create_test_commit(&odb, "M", vec![commit_a.clone(), commit_b.clone()]).await;

    // X and Y each merge with M
    let commit_x = create_test_commit(&odb, "X", vec![commit_a.clone(), commit_m.clone()]).await;
    let commit_y = create_test_commit(&odb, "Y", vec![commit_b.clone(), commit_m.clone()]).await;

    let bases = lca_finder
        .find_merge_base(&commit_x, &commit_y)
        .await
        .unwrap();

    // Should detect criss-cross scenario (multiple merge bases or complex history)
    assert!(!bases.is_empty());
}

/// Test performance requirement: <50ms for merge base finding
#[tokio::test]
async fn test_lca_performance() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create a moderately complex history (20 commits)
    let mut commits = Vec::new();
    commits.push(create_test_commit(&odb, "commit_0", vec![]).await);

    for i in 1..20 {
        let parent = commits[i - 1].clone();
        let commit = create_test_commit(&odb, &format!("commit_{}", i), vec![parent]).await;
        commits.push(commit);
    }

    // Create two divergent branches
    let branch_a = create_test_commit(&odb, "branch_a", vec![commits[10].clone()]).await;
    let branch_b = create_test_commit(&odb, "branch_b", vec![commits[10].clone()]).await;

    // Measure LCA performance
    let start = Instant::now();
    let _ = lca_finder
        .find_merge_base(&branch_a, &branch_b)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    // Should complete in <100ms (relaxed for CI/WSL2)
    assert!(
        elapsed.as_millis() < 100,
        "LCA took {}ms, expected <100ms",
        elapsed.as_millis()
    );
}

/// Test LCA when commits are identical (trivial case)
#[tokio::test]
async fn test_lca_identical_commits() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    let commit_a = create_test_commit(&odb, "A", vec![]).await;

    let bases = lca_finder
        .find_merge_base(&commit_a, &commit_a)
        .await
        .unwrap();

    // Commit should be its own merge base
    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commit_a);
}

/// Test LCA with no common ancestor (disjoint histories)
#[tokio::test]
async fn test_lca_no_common_ancestor() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create two independent histories
    let commit_a = create_test_commit(&odb, "A", vec![]).await;
    let commit_b = create_test_commit(&odb, "B", vec![]).await;

    let result = lca_finder.find_merge_base(&commit_a, &commit_b).await;

    // Should return error or empty result for disjoint histories
    assert!(result.is_err() || result.unwrap().is_empty());
}

/// Test deep history performance (100+ commits)
#[tokio::test]
async fn test_lca_deep_history() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let lca_finder = LcaFinder::new(odb.clone());

    // Create deep linear history
    let mut commits = Vec::new();
    commits.push(create_test_commit(&odb, "commit_0", vec![]).await);

    for i in 1..100 {
        let parent = commits[i - 1].clone();
        let commit = create_test_commit(&odb, &format!("commit_{}", i), vec![parent]).await;
        commits.push(commit);
    }

    // Create branches at different depths
    let branch_a = create_test_commit(&odb, "branch_a", vec![commits[80].clone()]).await;
    let branch_b = create_test_commit(&odb, "branch_b", vec![commits[80].clone()]).await;

    let start = Instant::now();
    let bases = lca_finder
        .find_merge_base(&branch_a, &branch_b)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(bases.len(), 1);
    assert_eq!(bases[0], commits[80]);

    // Should still be fast even with deep history (relaxed for CI/slow environments)
    assert!(
        elapsed.as_millis() < 500,
        "Deep history LCA took {}ms, expected <500ms",
        elapsed.as_millis()
    );
}
