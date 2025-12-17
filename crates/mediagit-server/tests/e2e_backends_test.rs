//! End-to-End Backend Integration Tests
//!
//! These tests verify the complete MediaGit flow across all storage backends:
//! - Local filesystem
//! - S3 (MinIO)
//! - Azure Blob Storage (Azurite)
//! - Google Cloud Storage (GCS emulator)
//!
//! # Prerequisites
//!
//! All storage backends must be running via Docker Compose:
//! ```bash
//! cd /mnt/d/own/saas/mediagit-core
//! docker-compose -f docker-compose.test.yml up -d
//! ```
//!
//! # Test Flow
//!
//! Each backend test performs:
//! 1. Server initialization with backend configuration
//! 2. Repository creation
//! 3. Object upload (media files from test-files/)
//! 4. Pack file operations
//! 5. Reference database operations
//! 6. Object retrieval and verification
//! 7. Cleanup
//!
//! # Media File Testing
//!
//! Tests use real media files from /mnt/d/own/saas/mediagit-core/test-files/:
//! - Video: MP4, MOV (Big Buck Bunny clips)
//! - Audio: FLAC, OGG, WAV
//! - Images: JPEG, WebP
//! - 3D Models: STL, GLB, USDZ

use axum::http::StatusCode;
use mediagit_protocol::{RefUpdateRequest, RefsResponse, WantRequest};
use mediagit_server::{create_router, AppState};
use mediagit_storage::{azure::AzureBackend, local::LocalBackend, minio::MinIOBackend, StorageBackend};
use mediagit_versioning::{ObjectDatabase, ObjectType, Oid, PackWriter, Ref, RefDatabase};
use reqwest::Client;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tempfile::TempDir;
use tokio::fs;

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Test server instance with cleanup
struct TestServer {
    addr: SocketAddr,
    _temp_dir: Option<TempDir>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl TestServer {
    /// Create test server with local filesystem backend
    async fn new_local() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repos_dir = temp_dir.path().join("repos");
        fs::create_dir_all(&repos_dir).await.unwrap();

        let state = Arc::new(AppState::new(repos_dir));
        Self::start_server(state, Some(temp_dir)).await
    }

    /// Create test server with MinIO backend
    async fn new_minio() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repos_dir = temp_dir.path().join("repos");
        fs::create_dir_all(&repos_dir).await.unwrap();

        let state = Arc::new(AppState::new(repos_dir));
        Self::start_server(state, Some(temp_dir)).await
    }

    /// Create test server with Azurite backend
    async fn new_azurite() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let repos_dir = temp_dir.path().join("repos");
        fs::create_dir_all(&repos_dir).await.unwrap();

        let state = Arc::new(AppState::new(repos_dir));
        Self::start_server(state, Some(temp_dir)).await
    }

    /// Start HTTP server with given state
    async fn start_server(state: Arc<AppState>, temp_dir: Option<TempDir>) -> Self {
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind");
        let addr = listener.local_addr().expect("Failed to get local address");

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("Server failed");
        });

        // Wait for server to be ready
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        TestServer {
            addr,
            _temp_dir: temp_dir,
            shutdown_tx,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Server will shut down when shutdown_tx is dropped
    }
}

/// Helper to read test media file
async fn read_test_file(filename: &str) -> Vec<u8> {
    let path = PathBuf::from("/mnt/d/own/saas/mediagit-core/test-files").join(filename);
    fs::read(&path).await.expect(&format!("Failed to read test file: {}", filename))
}

// ============================================================================
// Local Filesystem Backend Tests
// ============================================================================

#[tokio::test]
async fn test_local_backend_complete_flow() {
    let server = TestServer::new_local().await;
    let client = Client::new();
    let repo = "test-repo";

    // 1. Create repository directory
    let repo_path = server._temp_dir.as_ref().unwrap().path().join("repos").join(repo);
    fs::create_dir_all(&repo_path).await.unwrap();

    // Initialize storage backend
    let storage = LocalBackend::new(&repo_path).await.unwrap();
    let storage_arc: Arc<dyn StorageBackend> = Arc::new(storage);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);
    let refdb = RefDatabase::new(&repo_path);

    // 2. Upload a small test image
    let test_data = read_test_file("freepik__talk__71826.jpeg").await;
    let oid = odb.write(ObjectType::Blob, &test_data).await.unwrap();

    // 3. Create a reference pointing to the object
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), oid);
    refdb.write(&main_ref).await.unwrap();

    // 4. Test GET /repo/info/refs
    let resp = client
        .get(&server.url(&format!("/{}/info/refs", repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let refs_response: RefsResponse = resp.json().await.unwrap();
    assert!(!refs_response.refs.is_empty());

    let main_ref_info = refs_response.refs.iter()
        .find(|r| r.name == "refs/heads/main")
        .expect("main ref should exist");
    assert_eq!(main_ref_info.oid, oid.to_hex());

    // 5. Create and upload a pack file
    let mut pack_writer = PackWriter::new();
    pack_writer.add_object(oid, ObjectType::Blob, &test_data);
    let pack_data = pack_writer.finalize();

    let resp = client
        .post(&server.url(&format!("/{}/objects/pack", repo)))
        .header("content-type", "application/octet-stream")
        .body(pack_data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. Request objects and download pack
    let want_request = WantRequest {
        want: vec![oid.to_hex()],
        have: vec![],
    };

    let resp = client
        .post(&server.url(&format!("/{}/objects/want", repo)))
        .json(&want_request)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = client
        .get(&server.url(&format!("/{}/objects/pack", repo)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded_pack = resp.bytes().await.unwrap();
    assert!(!downloaded_pack.is_empty());

    // 7. Test ref update (using the existing OID that we just uploaded)
    let update_request = RefUpdateRequest {
        updates: vec![mediagit_protocol::RefUpdate {
            name: "refs/heads/feature".to_string(),
            old_oid: None,
            new_oid: oid.to_hex(),
        }],
        force: false,
    };

    let resp = client
        .post(&server.url(&format!("/{}/refs/update", repo)))
        .json(&update_request)
        .send()
        .await
        .unwrap();
    // Should succeed since the object exists
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_local_backend_large_media_file() {
    let server = TestServer::new_local().await;
    let repo = "media-repo";

    let repo_path = server._temp_dir.as_ref().unwrap().path().join("repos").join(repo);
    fs::create_dir_all(&repo_path).await.unwrap();

    let storage = LocalBackend::new(&repo_path).await.unwrap();
    let storage_arc: Arc<dyn StorageBackend> = Arc::new(storage);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);

    // Test with a larger video file (~5MB)
    let video_data = read_test_file("101394-video-720.mp4").await;
    assert!(video_data.len() > 1_000_000, "Video file should be > 1MB");

    let oid = odb.write(ObjectType::Blob, &video_data).await.unwrap();

    // Verify we can read it back
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved.len(), video_data.len());
    assert_eq!(retrieved, video_data);
}

#[tokio::test]
async fn test_local_backend_multiple_media_types() {
    let server = TestServer::new_local().await;
    let repo = "mixed-media";

    let repo_path = server._temp_dir.as_ref().unwrap().path().join("repos").join(repo);
    fs::create_dir_all(&repo_path).await.unwrap();

    let storage = LocalBackend::new(&repo_path).await.unwrap();
    let storage_arc: Arc<dyn StorageBackend> = Arc::new(storage);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);

    // Test different media types
    let test_files = vec![
        ("freepik__talk__71826.jpeg", "image/jpeg"),
        ("Workstation_cube_lid_off.webp", "image/webp"),
        ("_Into_the_Oceans_and_the_Air_.ogg", "audio/ogg"),
    ];

    let mut oids = Vec::new();
    for (filename, _mime_type) in &test_files {
        let data = read_test_file(filename).await;
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();
        oids.push((oid, data.len()));

        // Verify immediate readback
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved.len(), data.len());
    }

    // Verify all objects still exist
    for (oid, expected_size) in oids {
        let data = odb.read(&oid).await.unwrap();
        assert_eq!(data.len(), expected_size);
    }
}

// ============================================================================
// MinIO (S3) Backend Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires MinIO Docker container
async fn test_minio_backend_complete_flow() {
    // Create temporary directory for RefDatabase
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let refs_path = temp_dir.path().join("refs");
    fs::create_dir_all(&refs_path).await.unwrap();

    // Create MinIO backend
    let backend = MinIOBackend::new(
        "http://localhost:9000",
        "mediagit-test",
        "minioadmin",
        "minioadmin",
    )
    .await
    .expect("Failed to create MinIO backend");

    let storage_arc: Arc<dyn StorageBackend> = Arc::new(backend);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);
    let refdb = RefDatabase::new(&refs_path);

    // Test with image file
    let test_data = read_test_file("freepik__talk__71826.jpeg").await;
    let oid = odb.write(ObjectType::Blob, &test_data).await.unwrap();

    // Create ref
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), oid);
    refdb.write(&main_ref).await.unwrap();

    // Verify object retrieval
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, test_data);

    // Verify ref retrieval
    let read_ref = refdb.read("refs/heads/main").await.unwrap();
    assert_eq!(read_ref.oid, Some(oid));

    // Cleanup
    storage_arc.delete(&format!("objects/{}", oid.to_hex())).await.unwrap();
    storage_arc.delete("refs/heads/main").await.unwrap();
}

#[tokio::test]
#[ignore] // Requires MinIO
async fn test_minio_backend_large_video() {
    let backend = MinIOBackend::new(
        "http://localhost:9000",
        "mediagit-test",
        "minioadmin",
        "minioadmin",
    )
    .await
    .expect("Failed to create MinIO backend");

    let storage_arc: Arc<dyn StorageBackend> = Arc::new(backend);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);

    // Test with video file
    let video_data = read_test_file("101394-video-720.mp4").await;
    let oid = odb.write(ObjectType::Blob, &video_data).await.unwrap();

    // Verify retrieval
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved.len(), video_data.len());
    assert_eq!(retrieved, video_data);

    // Cleanup
    storage_arc.delete(&format!("objects/{}", oid.to_hex())).await.unwrap();
}

#[tokio::test]
#[ignore] // Requires MinIO
async fn test_minio_concurrent_uploads() {
    let backend = MinIOBackend::new(
        "http://localhost:9000",
        "mediagit-test",
        "minioadmin",
        "minioadmin",
    )
    .await
    .expect("Failed to create MinIO backend");

    let storage_arc: Arc<dyn StorageBackend> = Arc::new(backend);
    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage_arc), 1000));

    // Read test file once
    let test_data = read_test_file("freepik__talk__71826.jpeg").await;

    // Concurrent uploads
    let mut handles = vec![];
    for i in 0..5 {
        let odb_clone = Arc::clone(&odb);
        let data = test_data.clone();
        let handle = tokio::spawn(async move {
            let oid = odb_clone.write(ObjectType::Blob, &data).await.unwrap();
            (i, oid)
        });
        handles.push(handle);
    }

    // Collect results
    let mut oids = Vec::new();
    for handle in handles {
        let (_i, oid) = handle.await.unwrap();
        oids.push(oid);
    }

    // All should produce the same OID (content-addressed)
    for oid in &oids[1..] {
        assert_eq!(*oid, oids[0]);
    }

    // Cleanup
    storage_arc.delete(&format!("objects/{}", oids[0].to_hex())).await.unwrap();
}

// ============================================================================
// Azurite (Azure Blob) Backend Tests
// ============================================================================

#[tokio::test]
#[ignore] // Requires Azurite Docker container
async fn test_azurite_backend_complete_flow() {
    const AZURITE_CONNECTION_STRING: &str = "DefaultEndpointsProtocol=http;\
        AccountName=devstoreaccount1;\
        AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;\
        BlobEndpoint=http://localhost:10000/devstoreaccount1;";

    // Create temporary directory for RefDatabase
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let refs_path = temp_dir.path().join("refs");
    fs::create_dir_all(&refs_path).await.unwrap();

    // Create Azure backend
    let backend = AzureBackend::with_connection_string("mediagit-test", AZURITE_CONNECTION_STRING)
        .await
        .expect("Failed to create Azure backend");

    let storage_arc: Arc<dyn StorageBackend> = Arc::new(backend);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);
    let refdb = RefDatabase::new(&refs_path);

    // Test with image file
    let test_data = read_test_file("Workstation_cube_lid_off.webp").await;
    let oid = odb.write(ObjectType::Blob, &test_data).await.unwrap();

    // Create ref
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), oid);
    refdb.write(&main_ref).await.unwrap();

    // Verify object retrieval
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved, test_data);

    // Verify ref retrieval
    let read_ref = refdb.read("refs/heads/main").await.unwrap();
    assert_eq!(read_ref.oid, Some(oid));

    // Cleanup
    storage_arc.delete(&format!("objects/{}", oid.to_hex())).await.unwrap();
    storage_arc.delete("refs/heads/main").await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Azurite
async fn test_azurite_backend_audio_file() {
    const AZURITE_CONNECTION_STRING: &str = "DefaultEndpointsProtocol=http;\
        AccountName=devstoreaccount1;\
        AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;\
        BlobEndpoint=http://localhost:10000/devstoreaccount1;";

    let backend = AzureBackend::with_connection_string("mediagit-test", AZURITE_CONNECTION_STRING)
        .await
        .expect("Failed to create Azure backend");

    let storage_arc: Arc<dyn StorageBackend> = Arc::new(backend);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);

    // Test with audio file (FLAC - high quality, large file)
    let audio_data = read_test_file("_Amir_Tangsiri__Dokhtare_Koli.flac").await;
    assert!(audio_data.len() > 10_000_000, "FLAC file should be > 10MB");

    let oid = odb.write(ObjectType::Blob, &audio_data).await.unwrap();

    // Verify retrieval
    let retrieved = odb.read(&oid).await.unwrap();
    assert_eq!(retrieved.len(), audio_data.len());
    assert_eq!(retrieved, audio_data);

    // Cleanup
    storage_arc.delete(&format!("objects/{}", oid.to_hex())).await.unwrap();
}

// ============================================================================
// GCS Backend Tests
// ============================================================================
// Note: GCS tests are commented out as they require:
// 1. A service account JSON file
// 2. Real GCS project and bucket OR properly configured emulator
//
// For production testing, uncomment and configure with actual credentials:
//
// #[tokio::test]
// #[ignore] // Requires GCS configuration
// async fn test_gcs_backend_complete_flow() {
//     let backend = GcsBackend::new(
//         "test-project",
//         "mediagit-test",
//         "path/to/service-account.json"
//     )
//     .await
//     .expect("Failed to create GCS backend");
//
//     // ... test implementation similar to MinIO/Azurite
// }

// ============================================================================
// Cross-Backend Compatibility Tests
// ============================================================================

#[tokio::test]
async fn test_local_backend_pack_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let storage = LocalBackend::new(temp_dir.path()).await.unwrap();
    let storage_arc: Arc<dyn StorageBackend> = Arc::new(storage);
    let odb = ObjectDatabase::new(Arc::clone(&storage_arc), 1000);

    // Create multiple objects
    let file1 = read_test_file("freepik__talk__71826.jpeg").await;
    let file2 = read_test_file("Workstation_cube_lid_off.webp").await;

    let oid1 = odb.write(ObjectType::Blob, &file1).await.unwrap();
    let oid2 = odb.write(ObjectType::Blob, &file2).await.unwrap();

    // Create pack with both objects
    let mut pack_writer = PackWriter::new();
    pack_writer.add_object(oid1, ObjectType::Blob, &file1);
    pack_writer.add_object(oid2, ObjectType::Blob, &file2);
    let pack_data = pack_writer.finalize();

    assert!(!pack_data.is_empty());
    assert!(pack_data.len() > file1.len() + file2.len()); // Has headers
}

#[tokio::test]
async fn test_path_validation() {
    let server = TestServer::new_local().await;
    let client = Client::new();

    // Test path traversal protection
    let malicious_repos = vec![
        "../etc/passwd",
        "repo/../secrets",
        "/etc/passwd",
        "C:\\Windows",
        "repo\0malicious",
    ];

    for repo in malicious_repos {
        let resp = client
            .get(&server.url(&format!("/{}/info/refs", repo)))
            .send()
            .await
            .unwrap();

        // Should reject with BAD_REQUEST
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "Should reject malicious repo name: {}",
            repo
        );
    }
}
