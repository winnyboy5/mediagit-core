// End-to-end integration tests for push/pull workflow
// These tests start an actual server and test the complete client-server interaction

use std::sync::Arc;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{ObjectDatabase, RefDatabase, ObjectType, Ref, PackReader};
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

// Helper to initialize a test repository
async fn init_test_repo(repo_path: &std::path::Path) -> anyhow::Result<()> {
    let mediagit_dir = repo_path.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await?;
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await?;
    tokio::fs::create_dir_all(mediagit_dir.join("refs/heads")).await?;

    // Create initial commit
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(repo_path.to_path_buf()).await?);
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);

    // Write initial file
    let content = b"test file content";
    let oid = odb.write(ObjectType::Blob, content).await?;

    // Create initial ref
    let refdb = RefDatabase::new(Arc::clone(&storage));
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), oid);
    refdb.write(&main_ref).await?;

    // Set HEAD to point to main
    let head_ref = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
    refdb.write(&head_ref).await?;

    Ok(())
}

#[tokio::test]
async fn test_e2e_push_workflow() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository
    init_test_repo(&server_repo).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize client repository
    let client_repo = client_temp.path().to_path_buf();
    init_test_repo(&client_repo).await.unwrap();

    // Add new object to client
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client_repo.clone()).await.unwrap());
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let new_content = b"new test content for push";
    let new_oid = odb.write(ObjectType::Blob, new_content).await.unwrap();

    // Update client's main ref
    let refdb = RefDatabase::new(Arc::clone(&storage));
    let updated_ref = Ref::new_direct("refs/heads/main".to_string(), new_oid);
    refdb.write(&updated_ref).await.unwrap();

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Get current server state
    let refs_response = client.get_refs().await.unwrap();
    let old_oid = refs_response.refs.iter()
        .find(|r| r.name == "refs/heads/main")
        .and_then(|r| Some(r.oid.clone()));

    // Perform push
    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid,
        new_oid: new_oid.to_hex(),
    };

    let result = client.push(&odb, vec![update], false).await;

    // Verify push succeeded
    assert!(result.is_ok(), "Push failed: {:?}", result.err());
    let response = result.unwrap();
    assert!(response.success, "Push response indicates failure");
    assert_eq!(response.results.len(), 1);
    assert!(response.results[0].success);
    assert_eq!(response.results[0].ref_name, "refs/heads/main");

    // Verify server repository was updated
    let server_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(server_repo.clone()).await.unwrap());
    let server_refdb = RefDatabase::new(Arc::clone(&server_storage));
    let server_main = server_refdb.read("refs/heads/main").await.unwrap();

    if let Some(target_oid) = &server_main.oid {
        assert_eq!(target_oid.to_hex(), new_oid.to_hex(), "Server ref not updated correctly");
    } else {
        panic!("Server ref should be direct, not symbolic");
    }
}

#[tokio::test]
async fn test_e2e_pull_workflow() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository with content
    init_test_repo(&server_repo).await.unwrap();

    // Add additional object to server
    let server_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(server_repo.clone()).await.unwrap());
    let server_odb = ObjectDatabase::new(Arc::clone(&server_storage), 1000);
    let server_content = b"content from server to pull";
    let server_oid = server_odb.write(ObjectType::Blob, server_content).await.unwrap();

    // Update server's main ref
    let server_refdb = RefDatabase::new(Arc::clone(&server_storage));
    let server_ref = Ref::new_direct("refs/heads/main".to_string(), server_oid);
    server_refdb.write(&server_ref).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize empty client repository
    let client_repo = client_temp.path().to_path_buf();
    let mediagit_dir = client_repo.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("refs/heads")).await.unwrap();

    let client_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client_repo.clone()).await.unwrap());
    let client_odb = ObjectDatabase::new(Arc::clone(&client_storage), 1000);

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Perform pull
    let result = client.pull(&client_odb, "refs/heads/main").await;

    // Verify pull succeeded
    assert!(result.is_ok(), "Pull failed: {:?}", result.err());
    let pack_data = result.unwrap();

    // Unpack received objects into ODB
    let pack_reader = PackReader::new(pack_data).unwrap();
    for oid in pack_reader.list_objects() {
        let obj_data = pack_reader.get_object(&oid).unwrap();
        client_odb.write(ObjectType::Blob, &obj_data).await.unwrap();
    }

    // Verify object was downloaded
    let downloaded_obj = client_odb.read(&server_oid).await;
    assert!(downloaded_obj.is_ok(), "Downloaded object not found in client ODB");
    assert_eq!(downloaded_obj.unwrap(), server_content.to_vec());
}

#[tokio::test]
async fn test_e2e_push_then_pull_roundtrip() {
    // Setup temporary directories
    let server_temp = TempDir::new().unwrap();
    let client1_temp = TempDir::new().unwrap();
    let client2_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository
    init_test_repo(&server_repo).await.unwrap();

    // Start test server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // === Client 1: Push ===
    let client1_repo = client1_temp.path().to_path_buf();
    init_test_repo(&client1_repo).await.unwrap();

    // Add unique content to client1
    let client1_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client1_repo.clone()).await.unwrap());
    let client1_odb = ObjectDatabase::new(Arc::clone(&client1_storage), 1000);
    let unique_content = b"unique content from client1";
    let unique_oid = client1_odb.write(ObjectType::Blob, unique_content).await.unwrap();

    // Update client1's main ref
    let client1_refdb = RefDatabase::new(Arc::clone(&client1_storage));
    let client1_ref = Ref::new_direct("refs/heads/main".to_string(), unique_oid);
    client1_refdb.write(&client1_ref).await.unwrap();

    // Push from client1
    let client1_protocol = ProtocolClient::new(&format!("{}/test-repo", base_url));
    let refs_response = client1_protocol.get_refs().await.unwrap();
    let old_oid = refs_response.refs.iter()
        .find(|r| r.name == "refs/heads/main")
        .and_then(|r| Some(r.oid.clone()));

    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid,
        new_oid: unique_oid.to_hex(),
    };

    let push_result = client1_protocol.push(&client1_odb, vec![update], false).await;
    assert!(push_result.is_ok(), "Client1 push failed");
    assert!(push_result.unwrap().success);

    // === Client 2: Pull ===
    let client2_repo = client2_temp.path().to_path_buf();
    let mediagit_dir = client2_repo.join(".mediagit");
    tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
    tokio::fs::create_dir_all(mediagit_dir.join("objects")).await.unwrap();

    let client2_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client2_repo.clone()).await.unwrap());
    let client2_odb = ObjectDatabase::new(Arc::clone(&client2_storage), 1000);

    // Pull to client2
    let client2_protocol = ProtocolClient::new(&format!("{}/test-repo", base_url));
    let pull_result = client2_protocol.pull(&client2_odb, "refs/heads/main").await;
    assert!(pull_result.is_ok(), "Client2 pull failed");
    let pack_data = pull_result.unwrap();

    // Unpack received objects into client2's ODB
    let pack_reader = PackReader::new(pack_data).unwrap();
    for oid in pack_reader.list_objects() {
        let obj_data = pack_reader.get_object(&oid).unwrap();
        client2_odb.write(ObjectType::Blob, &obj_data).await.unwrap();
    }

    // Verify client2 received client1's content
    let client2_obj = client2_odb.read(&unique_oid).await;
    assert!(client2_obj.is_ok(), "Client2 should have client1's object");
    assert_eq!(client2_obj.unwrap(), unique_content.to_vec());
}

#[tokio::test]
async fn test_force_push() {
    // Setup
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();

    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("test-repo");
    tokio::fs::create_dir_all(&server_repo).await.unwrap();

    // Initialize server repository
    init_test_repo(&server_repo).await.unwrap();

    // Add content to server (simulating divergent history)
    let server_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(server_repo.clone()).await.unwrap());
    let server_odb = ObjectDatabase::new(Arc::clone(&server_storage), 1000);
    let server_content = b"server divergent content";
    let server_oid = server_odb.write(ObjectType::Blob, server_content).await.unwrap();

    let server_refdb = RefDatabase::new(Arc::clone(&server_storage));
    let server_ref = Ref::new_direct("refs/heads/main".to_string(), server_oid);
    server_refdb.write(&server_ref).await.unwrap();

    // Start server
    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // Initialize client with different history
    let client_repo = client_temp.path().to_path_buf();
    init_test_repo(&client_repo).await.unwrap();

    let client_storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(client_repo.clone()).await.unwrap());
    let client_odb = ObjectDatabase::new(Arc::clone(&client_storage), 1000);
    let client_content = b"client different content";
    let client_oid = client_odb.write(ObjectType::Blob, client_content).await.unwrap();

    let client_refdb = RefDatabase::new(Arc::clone(&client_storage));
    let client_ref = Ref::new_direct("refs/heads/main".to_string(), client_oid);
    client_refdb.write(&client_ref).await.unwrap();

    // Create protocol client
    let client = ProtocolClient::new(&format!("{}/test-repo", base_url));

    // Attempt normal push (should fail due to non-fast-forward)
    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: Some(server_oid.to_hex()),
        new_oid: client_oid.to_hex(),
    };

    let _normal_push = client.push(&client_odb, vec![update.clone()], false).await;
    // This might succeed or fail depending on server implementation
    // We'll test force push regardless

    // Force push (should succeed)
    let force_push = client.push(&client_odb, vec![update], true).await;
    assert!(force_push.is_ok(), "Force push failed: {:?}", force_push.err());

    let response = force_push.unwrap();
    assert!(response.success, "Force push should succeed");

    // Verify server was updated to client's version
    let server_refdb = RefDatabase::new(Arc::clone(&server_storage));
    let updated_ref = server_refdb.read("refs/heads/main").await.unwrap();

    if let Some(oid) = &updated_ref.oid {
        assert_eq!(oid.to_hex(), client_oid.to_hex(), "Server should have client's OID after force push");
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
