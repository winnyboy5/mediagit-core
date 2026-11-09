// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Integration tests for the hosted-repo admin routes:
//! GET/POST /admin/repos and DELETE /admin/repos/{name}.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use mediagit_security::auth::{ApiKeyAuth, JwtAuth, User, user::Role};
use mediagit_server::{AppState, create_router};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

struct Fx {
    state: Arc<AppState>,
    repos_dir: PathBuf,
    admin: String,
    write: String,
}

async fn fixture() -> Fx {
    fixture_with(None).await
}

async fn fixture_with(default_storage: Option<mediagit_config::StorageConfig>) -> Fx {
    let repos_dir = TempDir::new().unwrap().keep();
    let secret = "test-secret-key-for-admin-repos";
    let state = Arc::new(
        AppState::new_with_auth(repos_dir.clone(), secret, Arc::new(ApiKeyAuth::new()))
            .with_default_storage(default_storage),
    );
    for (id, role) in [("admin-user", Role::Admin), ("write-user", Role::Write)] {
        let user = User::new(id.into(), id.into(), format!("{id}@example.com"), role);
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();
    }
    let jwt = JwtAuth::new(secret);
    let admin = jwt
        .generate_token(
            "admin-user",
            ["repo:read", "repo:write", "repo:admin", "user:manage"]
                .map(String::from)
                .to_vec(),
        )
        .unwrap();
    let write = jwt
        .generate_token(
            "write-user",
            ["repo:read", "repo:write"].map(String::from).to_vec(),
        )
        .unwrap();
    Fx {
        state,
        repos_dir,
        admin,
        write,
    }
}

fn req(method: &str, uri: &str, token: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri).method(method);
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    match body {
        Some(j) => b
            .header("Content-Type", "application/json")
            .body(Body::from(j.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap()
}

async fn call(fx: &Fx, r: Request<Body>) -> (StatusCode, String) {
    let resp = create_router(Arc::clone(&fx.state))
        .oneshot(r)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn create(fx: &Fx, name: &str) -> StatusCode {
    call(
        fx,
        req(
            "POST",
            "/admin/repos",
            Some(&fx.admin),
            Some(&format!(r#"{{"name":"{name}"}}"#)),
        ),
    )
    .await
    .0
}

fn list_names(body: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn unauthenticated_is_401_and_non_admin_is_403() {
    let fx = fixture().await;
    for (m, u, b) in [
        ("GET", "/admin/repos", None),
        ("POST", "/admin/repos", Some(r#"{"name":"x"}"#)),
        ("DELETE", "/admin/repos/x?confirm=x", None),
    ] {
        assert_eq!(
            call(&fx, req(m, u, None, b)).await.0,
            StatusCode::UNAUTHORIZED,
            "{m} {u}"
        );
        assert_eq!(
            call(&fx, req(m, u, Some(&fx.write), b)).await.0,
            StatusCode::FORBIDDEN,
            "{m} {u}"
        );
    }
    // The non-admin POST must not have created anything.
    assert!(!fx.repos_dir.join("x").exists());
}

#[tokio::test]
async fn traversal_names_are_400_and_touch_no_fs() {
    let fx = fixture().await;
    let before: Vec<_> = std::fs::read_dir(&fx.repos_dir).unwrap().collect();
    for name in [
        "..", "a/b", "%2e%2e", "/abs", "C:evil", ".hidden", "a\\b", "",
    ] {
        let body = serde_json::json!({ "name": name }).to_string();
        let (s, _) = call(
            &fx,
            req("POST", "/admin/repos", Some(&fx.admin), Some(&body)),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "create {name:?}");
    }
    for seg in ["..", "%2e%2e", "C:evil", "%2fabs", "a%2fb"] {
        let uri = format!("/admin/repos/{seg}?confirm={seg}&force=true");
        let (s, _) = call(&fx, req("DELETE", &uri, Some(&fx.admin), None)).await;
        assert!(
            s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND,
            "delete {seg}: {s}"
        );
    }
    let after: Vec<_> = std::fs::read_dir(&fx.repos_dir).unwrap().collect();
    assert_eq!(before.len(), after.len());
}

#[tokio::test]
async fn create_twice_409_then_listed_and_served() {
    let fx = fixture().await;
    assert_eq!(create(&fx, "studio").await, StatusCode::CREATED);
    assert_eq!(create(&fx, "studio").await, StatusCode::CONFLICT);

    let (s, body) = call(&fx, req("GET", "/admin/repos", Some(&fx.admin), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list_names(&body), vec!["studio"]);

    // A created repo is a real repo: the server serves its refs.
    let (s, _) = call(&fx, req("GET", "/studio/info/refs", Some(&fx.admin), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(fx.repos_dir.join("studio/.mediagit/config.toml").exists());
}

#[tokio::test]
async fn delete_requires_matching_confirm() {
    let fx = fixture().await;
    create(&fx, "r1").await;
    for uri in ["/admin/repos/r1", "/admin/repos/r1?confirm=other"] {
        let (s, _) = call(&fx, req("DELETE", uri, Some(&fx.admin), None)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{uri}");
    }
    assert!(fx.repos_dir.join("r1").exists());
}

#[tokio::test]
async fn delete_non_empty_needs_force_then_list_is_empty() {
    let fx = fixture().await;
    create(&fx, "full").await;
    let heads = fx.repos_dir.join("full/.mediagit/refs/heads");
    std::fs::create_dir_all(&heads).unwrap();
    std::fs::write(heads.join("main"), "0".repeat(64)).unwrap();

    let (s, _) = call(
        &fx,
        req(
            "DELETE",
            "/admin/repos/full?confirm=full",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(fx.repos_dir.join("full").exists());

    let (s, _) = call(
        &fx,
        req(
            "DELETE",
            "/admin/repos/full?confirm=full&force=true",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = call(&fx, req("GET", "/admin/repos", Some(&fx.admin), None)).await;
    assert!(list_names(&body).is_empty());

    // Deleting an empty repo needs no force; a missing one is 404.
    create(&fx, "empty").await;
    let (s, body) = call(
        &fx,
        req(
            "DELETE",
            "/admin/repos/empty?confirm=empty",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["deleted"], true);
    assert_eq!(v["storage_retained"], false);
    assert_eq!(v["backend"], "filesystem");
    let (s, _) = call(
        &fx,
        req(
            "DELETE",
            "/admin/repos/empty?confirm=empty",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_refused_with_active_transfer_even_when_forced() {
    let fx = fixture().await;
    create(&fx, "busy").await;
    fx.state
        .want_cache
        .lock()
        .await
        .insert("req1".into(), "busy".into(), vec![], vec![]);
    let (s, _) = call(
        &fx,
        req(
            "DELETE",
            "/admin/repos/busy?confirm=busy&force=true",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(fx.repos_dir.join("busy").exists());
}

#[tokio::test]
async fn grant_listings_for_user_and_repo() {
    use mediagit_security::auth::GrantLevel;
    let fx = fixture().await;
    create(&fx, "g1").await;
    create(&fx, "g2").await;
    fx.state
        .grants
        .grant("write-user", "g2", GrantLevel::Write)
        .await
        .unwrap();
    fx.state
        .grants
        .grant("write-user", "g1", GrantLevel::Read)
        .await
        .unwrap();

    // Gate: 401 / 403 on both routes.
    for u in ["/auth/users/write-user/grants", "/admin/repos/g1/grants"] {
        assert_eq!(
            call(&fx, req("GET", u, None, None)).await.0,
            StatusCode::UNAUTHORIZED,
            "{u}"
        );
        assert_eq!(
            call(&fx, req("GET", u, Some(&fx.write), None)).await.0,
            StatusCode::FORBIDDEN,
            "{u}"
        );
    }

    let (s, body) = call(
        &fx,
        req(
            "GET",
            "/auth/users/write-user/grants",
            Some(&fx.admin),
            None,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v,
        serde_json::json!([{"repo":"g1","level":"read"},{"repo":"g2","level":"write"}])
    );

    let (s, body) = call(
        &fx,
        req("GET", "/admin/repos/g2/grants", Some(&fx.admin), None),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v,
        serde_json::json!([{"user_id":"write-user","username":"write-user","level":"write"}])
    );

    // Unknown user / repo / bad name.
    assert_eq!(
        call(
            &fx,
            req("GET", "/auth/users/nobody/grants", Some(&fx.admin), None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &fx,
            req("GET", "/admin/repos/nope/grants", Some(&fx.admin), None)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &fx,
            req("GET", "/admin/repos/%2e%2e/grants", Some(&fx.admin), None)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn user_listing_reports_disabled() {
    let fx = fixture().await;
    fx.state
        .auth_service()
        .unwrap()
        .credentials_store
        .set_disabled("write-user", true)
        .await
        .unwrap();
    let (s, body) = call(&fx, req("GET", "/auth/users", Some(&fx.admin), None)).await;
    assert_eq!(s, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let by = |id: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|u| u["id"] == id)
            .unwrap()["disabled"]
            .clone()
    };
    assert_eq!(by("write-user"), true);
    assert_eq!(by("admin-user"), false);
}

fn fs_storage(path: &std::path::Path) -> mediagit_config::StorageConfig {
    mediagit_config::StorageConfig::FileSystem(mediagit_config::FileSystemStorage {
        base_path: path.to_string_lossy().into_owned(),
        ..Default::default()
    })
}

fn repo_storage(fx: &Fx, name: &str) -> mediagit_config::StorageConfig {
    let path = fx.repos_dir.join(name).join(".mediagit/config.toml");
    let toml = std::fs::read_to_string(path).unwrap();
    toml::from_str::<mediagit_config::Config>(&toml)
        .unwrap()
        .storage
}

#[tokio::test]
async fn create_writes_server_default_storage_and_request_override_wins() {
    let (dflt, other) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let fx = fixture_with(Some(fs_storage(dflt.path()))).await;

    assert_eq!(create(&fx, "plain").await, StatusCode::CREATED);
    assert_eq!(repo_storage(&fx, "plain"), fs_storage(dflt.path()));

    let body = serde_json::json!({ "name": "custom", "storage": fs_storage(other.path()) });
    let r = req(
        "POST",
        "/admin/repos",
        Some(&fx.admin),
        Some(&body.to_string()),
    );
    assert_eq!(call(&fx, r).await.0, StatusCode::CREATED);
    assert_eq!(repo_storage(&fx, "custom"), fs_storage(other.path()));
}
