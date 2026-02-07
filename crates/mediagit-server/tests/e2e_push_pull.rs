// End-to-end integration tests for push/pull workflow
// These tests start an actual server and test the complete client-server interaction

use std::sync::Arc;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{ObjectDatabase, RefDatabase, ObjectType, Ref, PackReader, Commit, Tree, TreeEntry, FileMode, Signature, Oid};
use mediagit_protocol::{ProtocolClient, RefUpdate};

// Helper to create test server on random port
async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    // Create server state
    let state = Arc::new(mediagit_server::AppState::new(repos_dir.clone()));

    // Build router
    let app = mediagit_server::create_router(state);

    // Spawn server in background
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, handle)
}

// Helper to initialize a test repository with proper commit objects
async fn init_test_repo(repo_path: &std::path::Path) -> anyhow::Result<Oid> {
    let mediagit_dir = repo_path.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await?;
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await?;
    tokio::fs::create_dir_all(mediagit_dir.join("refs/heads")).await?;

    // Create ODB using .mediagit directory (matches server's storage location)
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(&mediagit_dir).await?);
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);

    // Write initial blob
    let content = b"test file content";
    let blob_oid = odb.write(ObjectType::Blob, content).await?;

    // Create tree containing the blob
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        "test.txt".to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(&odb).await?;

    // Create initial commit
    let author = Signature::now("Test User".to_string(), "test@example.com".to_string());
    let commit = Commit::new(tree_oid, author.clone(), author, "Initial commit".to_string());
    let commit_oid = commit.write(&odb).await?;

    // Create initial ref pointing to commit (not blob)
    let refdb = RefDatabase::new(&repo_path.join(".mediagit"));
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    refdb.write(&main_ref).await?;

    // Set HEAD to point to main
    let head_ref = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
    refdb.write(&head_ref).await?;

    Ok(commit_oid)
}

// Helper to create a new commit with updated content
async fn create_commit(
    odb: &ObjectDatabase,
    content: &[u8],
    filename: &str,
    message: &str,
    parent: Option<Oid>,
) -> anyhow::Result<Oid> {
    // Write blob
    let blob_oid = odb.write(ObjectType::Blob, content).await?;

    // Create tree
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        filename.to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(odb).await?;

    // Create commit
    let author = Signature::now("Test User".to_string(), "test@example.com".to_string());
    let mut commit = Commit::new(tree_oid, author.clone(), author, message.to_string());
    if let Some(p) = parent {
        commit.parents.push(p);
    }
    let commit_oid = commit.write(odb).await?;

    Ok(commit_oid)
}

/// Test end-to-end push workflow with proper commit objects
#[tokio::test]
async fn test_e2e_push_workflow() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository with proper commit
    let server_initial_oid = init_test_repo(&server_repo).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize client repository
    let client_repo = client_temp.path().to_path_buf();
    let _client_initial_oid = init_test_repo(&client_repo).await.unwrap();

    // Create new commit on client (use .mediagit to match init_test_repo)
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client_repo.join(".mediagit")).await.unwrap());
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let new_commit_oid = create_commit(
        &odb,
        b"new test content for push",
        "new_file.txt",
        "Add new file for push test",
        Some(server_initial_oid),
    ).await.unwrap();

    // Update client's main ref to point to new commit
    let refdb = RefDatabase::new(&client_repo.join(".mediagit"));
    let updated_ref = Ref::new_direct("refs/heads/main".to_string(), new_commit_oid);
    refdb.write(&updated_ref).await.unwrap();

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Get current server state
    let refs_response = client.get_refs().await.unwrap();
    let old_oid = refs_response.refs.iter()
        .find(|r| r.name == "refs/heads/main")
        .map(|r| r.oid.clone());

    // Perform push
    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid,
        new_oid: new_commit_oid.to_hex(),
    };

    let result = client.push(&odb, vec![update], false).await;

    // Verify push succeeded
    assert!(result.is_ok(), "Push failed: {:?}", result.err());
    let (response, _stats) = result.unwrap();
    assert!(response.success, "Push response indicates failure");
    assert_eq!(response.results.len(), 1);
    assert!(response.results[0].success);
    assert_eq!(response.results[0].ref_name, "refs/heads/main");

    // Verify server repository was updated
    let server_refdb = RefDatabase::new(&server_repo.join(".mediagit"));
    let server_main = server_refdb.read("refs/heads/main").await.unwrap();

    if let Some(target_oid) = &server_main.oid {
        assert_eq!(target_oid.to_hex(), new_commit_oid.to_hex(), "Server ref not updated correctly");
    } else {
        panic!("Server ref should be direct, not symbolic");
    }
}

/// Test end-to-end pull workflow with proper commit objects
#[tokio::test]
async fn test_e2e_pull_workflow() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository with proper commit
    let _initial_oid = init_test_repo(&server_repo).await.unwrap();

    // Add additional commit to server (use .mediagit path to match server's storage location)
    let server_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(server_repo.join(".mediagit")).await.unwrap());
    let server_odb = ObjectDatabase::new(Arc::clone(&server_storage), 1000);
    let server_commit_oid = create_commit(
        &server_odb,
        b"content from server to pull",
        "server_file.txt",
        "Server commit for pull test",
        Some(_initial_oid),
    ).await.unwrap();

    // Update server's main ref to new commit
    let server_refdb = RefDatabase::new(&server_repo.join(".mediagit"));
    let server_ref = Ref::new_direct("refs/heads/main".to_string(), server_commit_oid);
    server_refdb.write(&server_ref).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize empty client repository
    let client_repo = client_temp.path().to_path_buf();
    let mediagit_dir = client_repo.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("refs/heads")).await.unwrap();

    let client_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir.clone()).await.unwrap());
    let client_odb = ObjectDatabase::new(Arc::clone(&client_storage), 1000);

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Perform pull
    let result = client.pull(&client_odb, "refs/heads/main").await;

    // Verify pull succeeded
    assert!(result.is_ok(), "Pull failed: {:?}", result.err());
    let (pack_data, _oids) = result.unwrap();

    // Unpack received objects into ODB
    let pack_reader = PackReader::new(pack_data).unwrap();
    for oid in pack_reader.list_objects() {
        let (obj_type, obj_data) = pack_reader.get_object_with_type(&oid).unwrap();
        client_odb.write(obj_type, &obj_data).await.unwrap();
    }

    // Verify commit was downloaded
    let downloaded_commit = client_odb.read(&server_commit_oid).await;
    assert!(downloaded_commit.is_ok(), "Downloaded commit not found in client ODB");
}

/// Test push-then-pull roundtrip with proper commit objects
#[tokio::test]
async fn test_e2e_push_then_pull_roundtrip() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client1_temp = TempDir::new().unwrap();
    let client2_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository with proper commit
    let _server_initial_oid = init_test_repo(&server_repo).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // === Client 1: Push ===
    let client1_repo = client1_temp.path().to_path_buf();
    let client1_initial = init_test_repo(&client1_repo).await.unwrap();

    // Create unique commit on client1 (use .mediagit and client's own initial commit)
    let client1_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client1_repo.join(".mediagit")).await.unwrap());
    let client1_odb = ObjectDatabase::new(Arc::clone(&client1_storage), 1000);
    let unique_commit_oid = create_commit(
        &client1_odb,
        b"unique content from client1",
        "client1_file.txt",
        "Unique commit from client1",
        Some(client1_initial),  // Use client's own initial commit
    ).await.unwrap();

    // Update client1's main ref
    let client1_refdb = RefDatabase::new(&client1_repo.join(".mediagit"));
    let client1_ref = Ref::new_direct("refs/heads/main".to_string(), unique_commit_oid);
    client1_refdb.write(&client1_ref).await.unwrap();

    // Push from client1
    let client1_protocol = ProtocolClient::new(&format!("{}/test-repo", base_url));
    let refs_response = client1_protocol.get_refs().await.unwrap();
    let old_oid = refs_response.refs.iter()
        .find(|r| r.name == "refs/heads/main")
        .map(|r| r.oid.clone());

    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid,
        new_oid: unique_commit_oid.to_hex(),
    };

    let push_result = client1_protocol.push(&client1_odb, vec![update], false).await;
    assert!(push_result.is_ok(), "Client1 push failed: {:?}", push_result.err());
    assert!(push_result.unwrap().0.success);

    // === Client 2: Pull ===
    let client2_repo = client2_temp.path().to_path_buf();
    let mediagit_dir = client2_repo.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await.unwrap();

    let client2_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir.clone()).await.unwrap());
    let client2_odb = ObjectDatabase::new(Arc::clone(&client2_storage), 1000);

    // Pull to client2
    let client2_protocol = ProtocolClient::new(&format!("{}/test-repo", base_url));
    let pull_result = client2_protocol.pull(&client2_odb, "refs/heads/main").await;
    assert!(pull_result.is_ok(), "Client2 pull failed: {:?}", pull_result.err());
    let (pack_data, _oids) = pull_result.unwrap();

    // Unpack received objects into client2's ODB
    let pack_reader = PackReader::new(pack_data).unwrap();
    for oid in pack_reader.list_objects() {
        let (obj_type, obj_data) = pack_reader.get_object_with_type(&oid).unwrap();
        client2_odb.write(obj_type, &obj_data).await.unwrap();
    }

    // Verify client2 received client1's commit
    let client2_commit = client2_odb.read(&unique_commit_oid).await;
    assert!(client2_commit.is_ok(), "Client2 should have client1's commit");
}

/// Test force push with divergent histories using proper commit objects
#[tokio::test]
async fn test_force_push() {
    // Setup
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository with proper commit
    let server_initial_oid = init_test_repo(&server_repo).await.unwrap();

    // Add divergent commit to server (simulating divergent history)
    let server_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(server_repo.join(".mediagit")).await.unwrap());
    let server_odb = ObjectDatabase::new(Arc::clone(&server_storage), 1000);
    let server_divergent_oid = create_commit(
        &server_odb,
        b"server divergent content",
        "server_divergent.txt",
        "Server divergent commit",
        Some(server_initial_oid),
    ).await.unwrap();

    let server_refdb = RefDatabase::new(&server_repo.join(".mediagit"));
    let server_ref = Ref::new_direct("refs/heads/main".to_string(), server_divergent_oid);
    server_refdb.write(&server_ref).await.unwrap();

    // Start server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize client with its OWN base commit (not using server's commit as parent)
    // This simulates a truly divergent history where client started from scratch
    let client_repo = client_temp.path().to_path_buf();
    let client_initial = init_test_repo(&client_repo).await.unwrap();

    let client_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client_repo.join(".mediagit")).await.unwrap());
    let client_odb = ObjectDatabase::new(Arc::clone(&client_storage), 1000);

    // Create client's divergent commit based on its own initial commit
    let client_divergent_oid = create_commit(
        &client_odb,
        b"client different content",
        "client_divergent.txt",
        "Client divergent commit",
        Some(client_initial),  // Use client's own initial commit as parent
    ).await.unwrap();

    let client_refdb = RefDatabase::new(&client_repo.join(".mediagit"));
    let client_ref = Ref::new_direct("refs/heads/main".to_string(), client_divergent_oid);
    client_refdb.write(&client_ref).await.unwrap();

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Force push (should succeed since we're forcing)
    // Note: We pass None for old_oid since we're forcing a complete overwrite
    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: None,  // Force push doesn't check old OID
        new_oid: client_divergent_oid.to_hex(),
    };

    let force_push = client.push(&client_odb, vec![update], true).await;
    assert!(force_push.is_ok(), "Force push failed: {:?}", force_push.err());

    let (response, _stats) = force_push.unwrap();
    assert!(response.success, "Force push should succeed");

    // Verify server was updated to client's version
    let server_refdb = RefDatabase::new(&server_repo.join(".mediagit"));
    let updated_ref = server_refdb.read("refs/heads/main").await.unwrap();

    if let Some(oid) = &updated_ref.oid {
        assert_eq!(oid.to_hex(), client_divergent_oid.to_hex(), "Server should have client's OID after force push");
    }
}

#[tokio::test]
async fn test_list_refs() {
    // Setup
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize repository
    init_test_repo(&server_repo).await.unwrap();

    // Start server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Create client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // List refs
    let result = client.get_refs().await;
    assert!(result.is_ok(), "List refs failed: {:?}", result.err());

    let response = result.unwrap();

    // Verify response structure
    assert!(response.refs.len() > 0, "Should have at least one ref");
    assert!(response.capabilities.contains(&"pack-v1".to_string()));

    // Check for expected refs
    let has_main = response.refs.iter().any(|r| r.name == "refs/heads/main");
    assert!(has_main, "Should have refs/heads/main");
}
