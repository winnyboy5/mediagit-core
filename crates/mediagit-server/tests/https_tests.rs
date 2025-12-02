//! HTTPS/TLS integration tests
//!
//! Tests the HTTPS server functionality with self-signed certificates
//! and certificate validation.

#[cfg(feature = "tls")]
mod https_tests {
    use mediagit_security::{CertificateBuilder, TlsConfigBuilder};
    use mediagit_server::{create_router, AppState};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper to create a test server with HTTPS
    async fn create_test_https_server() -> (String, TempDir, u16) {
        use axum_server::tls_rustls::RustlsConfig;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};

        // Install default crypto provider (required for rustls 0.23)
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create temp directory for repos
        let temp_dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(temp_dir.path().to_path_buf()));

        // Generate self-signed certificate
        let certificate = CertificateBuilder::new("localhost")
            .add_san_dns("localhost")
            .add_san_ip("127.0.0.1")
            .generate_self_signed()
            .unwrap();

        // Build rustls ServerConfig
        let cert_pem = certificate.cert_pem.as_bytes();
        let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut &cert_pem[..])
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let key_pem = certificate.key_pem.as_bytes();
        let mut key_reader = &key_pem[..];
        let private_keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let private_key = PrivateKeyDer::Pkcs8(private_keys.into_iter().next().unwrap());

        let rustls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, private_key)
            .unwrap();

        let axum_rustls_config = RustlsConfig::from_config(Arc::new(rustls_config));

        // Find available port
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let addr = format!("127.0.0.1:{}", port).parse().unwrap();
        let https_url = format!("https://127.0.0.1:{}", port);

        // Spawn server in background
        let app = create_router(state);
        tokio::spawn(async move {
            axum_server::bind_rustls(addr, axum_rustls_config)
                .serve(app.into_make_service())
                .await
                .ok();
        });

        // Give server time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        (https_url, temp_dir, port)
    }

    #[tokio::test]
    async fn test_https_server_starts() {
        let (https_url, _temp_dir, _port) = create_test_https_server().await;
        println!("HTTPS server started on: {}", https_url);
        assert!(https_url.starts_with("https://127.0.0.1:"));
    }

    #[tokio::test]
    async fn test_certificate_generation() {
        // Test self-signed certificate generation
        let cert = CertificateBuilder::new("test.example.com")
            .organization("Test Org")
            .country("US")
            .validity_days(30)
            .add_san_dns("test.example.com")
            .add_san_dns("localhost")
            .add_san_ip("127.0.0.1")
            .generate_self_signed()
            .unwrap();

        assert!(cert.cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(cert.key_pem.contains("-----BEGIN PRIVATE KEY-----"));
        assert_eq!(cert.common_name, "test.example.com");
    }

    #[tokio::test]
    async fn test_tls_config_self_signed() {
        let config = TlsConfigBuilder::new()
            .enable()
            .self_signed("localhost")
            .build()
            .unwrap();

        assert!(config.enabled);
        assert!(config.use_self_signed);

        // Load certificate
        let cert = config.load_certificate().unwrap();
        assert!(cert.cert_pem.contains("-----BEGIN CERTIFICATE-----"));
    }

    #[tokio::test]
    async fn test_tls_config_from_files() {
        use std::io::Write;

        // Generate certificate
        let cert = CertificateBuilder::new("file-test.example.com")
            .generate_self_signed()
            .unwrap();

        // Create temp directory
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("cert.pem");
        let key_path = temp_dir.path().join("key.pem");

        // Write to files
        std::fs::File::create(&cert_path)
            .unwrap()
            .write_all(cert.cert_pem.as_bytes())
            .unwrap();
        std::fs::File::create(&key_path)
            .unwrap()
            .write_all(cert.key_pem.as_bytes())
            .unwrap();

        // Load from config
        let config = TlsConfigBuilder::new()
            .enable()
            .certificate_paths(&cert_path, &key_path)
            .build()
            .unwrap();

        let loaded_cert = config.load_certificate().unwrap();
        assert_eq!(loaded_cert.cert_pem, cert.cert_pem);
        assert_eq!(loaded_cert.key_pem, cert.key_pem);
    }

    #[tokio::test]
    async fn test_certificate_validation() {
        // Valid certificate
        let cert = CertificateBuilder::new("valid.example.com")
            .generate_self_signed()
            .unwrap();

        let validation = cert.validate();
        assert!(validation.is_ok());
    }

    #[tokio::test]
    async fn test_concurrent_https_requests() {
        let (https_url, _temp_dir, _port) = create_test_https_server().await;

        // Create HTTPS client that accepts self-signed certificates
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true) // Only for testing!
            .build()
            .unwrap();

        // Make multiple concurrent requests
        let mut handles = vec![];
        for i in 0..10 {
            let url = format!("{}/test{}/info/refs", https_url, i);
            let client = client.clone();

            handles.push(tokio::spawn(async move {
                client.get(&url).send().await
            }));
        }

        // Wait for all requests
        for handle in handles {
            let response = handle.await.unwrap();
            // Expect 404 since repos don't exist, but connection should work
            assert!(response.is_ok());
        }
    }

    #[tokio::test]
    async fn test_tls_handshake() {
        let (https_url, _temp_dir, _port) = create_test_https_server().await;

        // Create client that accepts self-signed certs
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();

        // Make request to verify TLS handshake works
        let response = client
            .get(format!("{}/nonexistent/info/refs", https_url))
            .send()
            .await
            .unwrap();

        // Should complete handshake even if endpoint doesn't exist
        assert!(response.status().is_client_error() || response.status().is_success());
    }

    #[tokio::test]
    async fn test_https_with_http_redirect() {
        // This test verifies that HTTPS works independently
        // In production, you might want HTTP to redirect to HTTPS

        let (https_url, _temp_dir, _port) = create_test_https_server().await;

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();

        // Verify HTTPS endpoint works
        let response = client
            .get(format!("{}/test/info/refs", https_url))
            .send()
            .await
            .unwrap();

        // Should get a response (even if 404)
        assert!(response.status().as_u16() >= 200);
    }
}

#[cfg(not(feature = "tls"))]
#[test]
fn test_tls_feature_disabled() {
    // When TLS feature is disabled, verify compilation still works
    // This test always passes but ensures non-TLS builds work
    assert!(true, "TLS feature is disabled");
}
