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

//! Minimal admin surface (H3): user, grant, and API-key management.
//!
//! Every handler here is gated on the flat `user:manage` permission (only
//! `Role::Admin` carries it — see `User::permissions`). The `repo` argument
//! `check_permission` normally uses for per-repo grant lookups is irrelevant
//! for `user:manage`: that permission string isn't repo-scoped, so
//! `check_permission` always falls back to the flat role check for it
//! regardless of what `repo` is passed (see `handlers::check_permission`
//! doc comment) — these handlers pass `""`.
//!
//! Grant mutations go through `state.grants`, which is now the only
//! `GrantsStore` in the process — the same instance `check_permission` reads
//! for repo-level enforcement. `AuthService` previously carried a second,
//! never-read instance that agreed with this one only at boot; writing
//! through it would have left a granted user without access until a restart.
//! It was deleted rather than documented (AU-9), so there is no longer a
//! wrong instance to pick.

use super::*;
use mediagit_security::auth::{
    ApiKey, User, user::Role, validate_password_strength, validate_registration_input,
};
use serde::{Deserialize, Serialize};

/// User info for the admin listing — deliberately excludes the password
/// hash (which isn't even on `User`; it lives on `UserCredentials`) and
/// email, matching exactly what was asked: id, username, role.
#[derive(Serialize)]
pub struct AdminUserInfo {
    pub id: String,
    pub username: String,
    pub role: Role,
}

impl From<User> for AdminUserInfo {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
        }
    }
}

/// API key info for the admin listing — metadata only, never the plaintext
/// key (which isn't stored anywhere after generation) or its hash.
#[derive(Serialize)]
pub struct AdminKeyInfo {
    pub id: String,
    pub name: String,
    pub user_id: String,
    pub created_at: i64,
}

impl From<ApiKey> for AdminKeyInfo {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id,
            name: k.name,
            user_id: k.user_id,
            created_at: k.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct GrantRequest {
    pub repo: String,
    pub level: GrantLevel,
}

#[derive(Deserialize)]
pub struct GrantRepoRequest {
    pub repo: String,
}

/// GET /auth/users — list all users (id, username, role only).
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminUserInfo>>, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    let users = auth_service.credentials_store.list_users().await;
    Ok(Json(users.into_iter().map(AdminUserInfo::from).collect()))
}

/// DELETE /auth/users/{id} — remove a user account and any grants they held.
pub async fn delete_user(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<StatusCode, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    auth_service
        .credentials_store
        .delete_user(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .grants
        .remove_user(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // AU-1: revoke the user's API keys as part of deletion.
    //
    // Credentials and grants were cleared but keys were not, and `ApiKey` has
    // no expiry — so a deleted user's keys kept authenticating indefinitely.
    // Deletion is frequently *how* a compromised or offboarded account is
    // handled, which made this the gap most likely to be relied upon.
    if let Some(layer) = state.auth_layer.as_ref() {
        let revoked = layer
            .api_key_auth()
            .revoke_user_keys(&id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if revoked > 0 {
            tracing::info!(user_id = %id, revoked, "revoked API keys for deleted user");
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /auth/users/{id}/grants — upsert a per-repo grant for the user.
pub async fn upsert_grant(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<GrantRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    state
        .grants
        .grant(&id, &req.repo, req.level)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /auth/users/{id}/grants — remove a per-repo grant for the user.
pub async fn remove_grant(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<GrantRepoRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    state
        .grants
        .revoke(&id, &req.repo)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /auth/keys — list all API keys (metadata only, never the secret).
pub async fn list_keys(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminKeyInfo>>, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;
    let keys = auth_layer.api_key_auth().list_all_keys().await;
    Ok(Json(keys.into_iter().map(AdminKeyInfo::from).collect()))
}

/// DELETE /auth/keys/{id} — revoke an API key. An admin (`user:manage`) may
/// revoke any key; any other authenticated user may only revoke a key they
/// own themselves.
pub async fn revoke_key(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<StatusCode, StatusCode> {
    let caller = auth_user.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;

    let is_admin = caller.permissions.contains(&"user:manage".to_string());
    if !is_admin {
        let owns_key = auth_layer
            .api_key_auth()
            .list_user_keys(&caller.user_id)
            .await
            .iter()
            .any(|k| k.id == id);
        if !owns_key {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    auth_layer
        .api_key_auth()
        .revoke_key(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    pub name: String,
    /// Requested permissions; capped to the key owner's own permission set
    /// (see below) so a key can never grant more than its owner already
    /// has. Defaults to the owner's full permission set when omitted.
    #[serde(default)]
    pub permissions: Option<Vec<String>>,
    /// Admin-only: mint the key for this user instead of the caller.
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Serialize)]
pub struct CreateKeyResponse {
    pub id: String,
    /// Plaintext key — returned exactly once; it is never stored or
    /// retrievable again after this response.
    pub key: String,
    pub name: String,
    pub permissions: Vec<String>,
}

/// POST /auth/keys — mint a new API key for the caller, or (admin only, via
/// `user_id`) for another user. Requested permissions are always capped to
/// the key owner's own role permissions, so a non-admin caller cannot mint a
/// key with broader access than their own account has, and an admin cannot
/// mint a key for someone else that exceeds that other user's role either.
pub async fn create_key(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CreateKeyRequest>,
) -> Result<(StatusCode, Json<CreateKeyResponse>), StatusCode> {
    let caller = auth_user.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;

    let owner_id = match &req.user_id {
        Some(uid) if uid != &caller.user_id => {
            check_permission(
                Some(caller),
                "user:manage",
                state.is_auth_enabled(),
                &state.grants,
                "",
            )?;
            uid.clone()
        }
        _ => caller.user_id.clone(),
    };

    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    let owner = auth_service
        .credentials_store
        .get_user(&owner_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let owner_permissions = owner.permissions();

    let requested = req.permissions.unwrap_or_else(|| owner_permissions.clone());
    let granted: Vec<String> = requested
        .into_iter()
        .filter(|p| owner_permissions.contains(p))
        .collect();

    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;
    let (plaintext_key, api_key) = auth_layer
        .api_key_auth()
        .generate_key(owner_id, req.name, granted)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateKeyResponse {
            id: api_key.id,
            key: plaintext_key,
            name: api_key.name,
            permissions: api_key.permissions,
        }),
    ))
}

/// GET /auth/keys/mine — list the caller's own API keys (metadata only).
pub async fn list_my_keys(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminKeyInfo>>, StatusCode> {
    let caller = auth_user.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;
    let keys = auth_layer
        .api_key_auth()
        .list_user_keys(&caller.user_id)
        .await;
    Ok(Json(keys.into_iter().map(AdminKeyInfo::from).collect()))
}

/// Response note appended wherever a role or password change won't take
/// effect on already-issued tokens: JWT claims embed permissions with a 24h
/// TTL and are never revoked (see `jwt.rs`), so a stolen or stale token
/// keeps working until it naturally expires.
const NO_REVOCATION_NOTE: &str = "This does not invalidate existing sessions - JWTs are valid for up to 24h after issue and are not revoked by this change.";

#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub role: Role,
}

#[derive(Serialize)]
pub struct SetRoleResponse {
    pub id: String,
    pub role: Role,
    pub note: String,
}

/// PATCH /auth/users/{id}/role — change a user's role. Refuses to demote the
/// last remaining Admin, since that would lock the server out of its own
/// admin surface with no HTTP-reachable way back in.
pub async fn set_role(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<SetRoleRequest>,
) -> Result<Json<SetRoleResponse>, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;

    let target = auth_service
        .credentials_store
        .get_user(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if target.role == Role::Admin && req.role != Role::Admin {
        let admin_count = auth_service
            .credentials_store
            .count_by_role(Role::Admin)
            .await;
        if admin_count <= 1 {
            return Err(StatusCode::CONFLICT);
        }
    }

    auth_service
        .credentials_store
        .set_role(&id, req.role)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(SetRoleResponse {
        id,
        role: req.role,
        note: NO_REVOCATION_NOTE.to_string(),
    }))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Serialize)]
pub struct ChangePasswordResponse {
    pub note: String,
}

/// POST /auth/password — self-service password change. Requires the
/// caller's current password before accepting the new one.
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, StatusCode> {
    let caller = auth_user.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;

    validate_password_strength(&req.new_password).map_err(|_| StatusCode::BAD_REQUEST)?;

    let verified = auth_service
        .credentials_store
        .verify_password(&caller.user_id, &req.current_password)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !verified {
        return Err(StatusCode::UNAUTHORIZED);
    }

    auth_service
        .credentials_store
        .update_password(&caller.user_id, &req.new_password)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ChangePasswordResponse {
        note: NO_REVOCATION_NOTE.to_string(),
    }))
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub new_password: String,
}

/// PATCH /auth/users/{id}/password — admin password reset. No current
/// password required (this is the forgot-password recovery path).
pub async fn reset_password(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    validate_password_strength(&req.new_password).map_err(|_| StatusCode::BAD_REQUEST)?;

    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    auth_service
        .credentials_store
        .update_password(&id, &req.new_password)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub role: Role,
}

/// POST /auth/users — admin creates a user with an explicit role. This is
/// what makes closed registration (`allow_open_registration = false`)
/// usable: accounts come from an admin instead of self-service register.
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<AdminUserInfo>), StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    validate_registration_input(&req.username, &req.email, &req.password)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    let user_id = uuid::Uuid::new_v4().to_string();
    let creds = auth_service
        .credentials_store
        .create_user_with_role(user_id, req.username, req.email, &req.password, req.role)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;

    Ok((StatusCode::CREATED, Json(AdminUserInfo::from(creds.user))))
}

#[derive(Serialize)]
pub struct GrantInfo {
    pub repo: String,
    pub level: GrantLevel,
}

#[derive(Serialize)]
pub struct WhoAmIResponse {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: Role,
    pub permissions: Vec<String>,
    /// The caller's own per-repo grants, read from the live `GrantsStore`
    /// (`state.grants` — the instance `check_permission` actually enforces
    /// against, not `AuthService::grants_store`, which only agrees with it
    /// at boot). Without this, a 403 is unexplainable from the client side.
    pub grants: Vec<GrantInfo>,
}

/// GET /auth/whoami — the caller's own identity, role, and per-repo grants.
pub async fn whoami(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<WhoAmIResponse>, StatusCode> {
    let caller = auth_user.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    let user = auth_service
        .credentials_store
        .get_user(&caller.user_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let grants = state
        .grants
        .list_for_user(&caller.user_id)
        .into_iter()
        .map(|(repo, level)| GrantInfo { repo, level })
        .collect();
    let permissions = user.permissions();

    Ok(Json(WhoAmIResponse {
        id: user.id,
        username: user.username,
        email: user.email,
        role: user.role,
        permissions,
        grants,
    }))
}
