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

// Regression test for BUG-V10-MERGE-PARTIAL
// When merge produces conflicts, workdir must get conflict markers + clean additions staged.

use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{
    apply_merge_to_workdir, Conflict, ConflictSide, ConflictType, FileMode, Index, MergeResult,
    MergeStrategy, ObjectDatabase, ObjectType, Oid, Tree, TreeEntry,
};
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_odb() -> Arc<ObjectDatabase> {
    let storage = Arc::new(MockBackend::new());
    Arc::new(ObjectDatabase::new(storage, 100))
}

async fn write_blob(odb: &Arc<ObjectDatabase>, content: &[u8]) -> Oid {
    odb.write(ObjectType::Blob, content).await.unwrap()
}

async fn create_tree_with_entries(
    odb: &Arc<ObjectDatabase>,
    entries: Vec<(&str, &[u8])>,
) -> (Tree, Oid) {
    let mut tree = Tree::new();
    for (name, content) in entries {
        let oid = write_blob(odb, content).await;
        let entry = TreeEntry::new(name.to_string(), FileMode::Regular, oid);
        tree.add_entry(entry);
    }
    let oid = tree.write(odb).await.unwrap();
    (tree, oid)
}

#[tokio::test]
async fn merge_conflict_writes_markers_and_stages_clean_paths() {
    let odb = create_test_odb();
    let tmpdir = TempDir::new().unwrap();
    let workdir = tmpdir.path();

    // Create .mediagit directory
    let mediagit_dir = workdir.join(".mediagit");
    std::fs::create_dir_all(&mediagit_dir).unwrap();

    // Base tree: README.md (3 lines) + shared.txt
    let base_readme = b"line1\nbase-line2\nline3\n";
    let base_shared = b"shared content\n";
    let (base_tree, _) = create_tree_with_entries(
        &odb,
        vec![("README.md", base_readme), ("shared.txt", base_shared)],
    )
    .await;

    // Ours tree: README.md line2 changed + a-only.txt added
    let ours_readme = b"line1\nours-line2\nline3\n";
    let ours_aonly = b"a-only content\n";
    let (ours_tree, _) = create_tree_with_entries(
        &odb,
        vec![
            ("README.md", ours_readme),
            ("shared.txt", base_shared),
            ("a-only.txt", ours_aonly),
        ],
    )
    .await;

    // Theirs tree: README.md line2 changed differently + b-only.txt added
    let theirs_readme = b"line1\ntheirs-line2\nline3\n";
    let theirs_bonly = b"b-only content\n";
    let (theirs_tree, _) = create_tree_with_entries(
        &odb,
        vec![
            ("README.md", theirs_readme),
            ("shared.txt", base_shared),
            ("b-only.txt", theirs_bonly),
        ],
    )
    .await;

    // Build conflict entry for README.md
    let base_readme_oid = base_tree.entries.get("README.md").unwrap().oid;
    let ours_readme_oid = ours_tree.entries.get("README.md").unwrap().oid;
    let theirs_readme_oid = theirs_tree.entries.get("README.md").unwrap().oid;

    let conflict = Conflict {
        path: "README.md".to_string(),
        conflict_type: ConflictType::ModifyModify,
        base: Some(ConflictSide {
            oid: base_readme_oid,
            mode: 0o100644,
        }),
        ours: Some(ConflictSide {
            oid: ours_readme_oid,
            mode: 0o100644,
        }),
        theirs: Some(ConflictSide {
            oid: theirs_readme_oid,
            mode: 0o100644,
        }),
    };

    let theirs_tip = Oid::hash(b"theirs-tip-sentinel");
    let orig_head = Oid::hash(b"orig-head-sentinel");

    let merge_result = MergeResult {
        tree_oid: None,
        conflicts: vec![conflict],
        success: false,
        fast_forward: None,
        strategy: MergeStrategy::Recursive,
    };

    let mut index = Index::new();

    apply_merge_to_workdir(
        &merge_result,
        &ours_tree,
        &theirs_tree,
        &odb,
        workdir,
        &mut index,
        theirs_tip,
        orig_head,
    )
    .await
    .unwrap();

    // (a) README.md has conflict markers
    let readme_content = std::fs::read_to_string(workdir.join("README.md")).unwrap();
    assert!(
        readme_content.contains("<<<<<<< ours"),
        "Missing <<<<<<< ours marker"
    );
    assert!(readme_content.contains("======="), "Missing ======= marker");
    assert!(
        readme_content.contains(">>>>>>> theirs"),
        "Missing >>>>>>> theirs marker"
    );
    assert!(
        readme_content.contains("ours-line2"),
        "Missing ours version of line2"
    );
    assert!(
        readme_content.contains("theirs-line2"),
        "Missing theirs version of line2"
    );

    // (b) a-only.txt and b-only.txt both present in workdir
    assert!(
        workdir.join("a-only.txt").exists(),
        "a-only.txt not in workdir"
    );
    assert!(
        workdir.join("b-only.txt").exists(),
        "b-only.txt not in workdir"
    );

    // a-only.txt and b-only.txt staged in index at stage 0
    let a_only_staged = index
        .entries()
        .any(|e| e.path.to_string_lossy() == "a-only.txt");
    let b_only_staged = index
        .entries()
        .any(|e| e.path.to_string_lossy() == "b-only.txt");
    assert!(a_only_staged, "a-only.txt not staged in index");
    assert!(b_only_staged, "b-only.txt not staged in index");

    // (c) MERGE_HEAD written with theirs_tip OID
    let merge_head_path = mediagit_dir.join("MERGE_HEAD");
    assert!(merge_head_path.exists(), "MERGE_HEAD not written");
    let merge_head_content = std::fs::read_to_string(&merge_head_path).unwrap();
    assert_eq!(
        merge_head_content.trim(),
        theirs_tip.to_hex(),
        "MERGE_HEAD content mismatch"
    );

    // (d) MERGE_MSG and ORIG_HEAD exist
    assert!(
        mediagit_dir.join("MERGE_MSG").exists(),
        "MERGE_MSG not written"
    );
    assert!(
        mediagit_dir.join("ORIG_HEAD").exists(),
        "ORIG_HEAD not written"
    );

    let orig_head_content = std::fs::read_to_string(mediagit_dir.join("ORIG_HEAD")).unwrap();
    assert_eq!(
        orig_head_content.trim(),
        orig_head.to_hex(),
        "ORIG_HEAD content mismatch"
    );
}
