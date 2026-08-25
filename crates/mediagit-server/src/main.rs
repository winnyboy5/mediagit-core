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

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use mediagit_server::{
    AppState, RateLimitConfig, ServerConfig, create_rate_limited_router, create_router,
    create_router_sharing_rate_limit,
};

mod setup;

/// MediaGit Server - HTTP(S) server for MediaGit repositories
#[derive(Parser, Debug)]
#[command(name = "mediagit-server")]
#[command(about = "MediaGit repository server", long_about = None)]
struct Args {
    /// Setup subcommands (init / admin). Bare `mediagit-server` (no
    /// subcommand) still means "serve", exactly as before.
    #[command(subcommand)]
    command: Option<Cmd>,

    /// Port to listen on (overrides config file)
    #[arg(short, long)]
    port: Option<u16>,

    /// Host address to bind to (overrides config file)
    #[arg(long)]
    host: Option<String>,

    /// Directory for repository storage (overrides config file repos_dir)
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Path to config file
    #[arg(short, long, default_value = "mediagit-server.toml")]
    config: String,
}

#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Interactive setup wizard: writes mediagit-server.toml and (optionally)
    /// creates the first admin user.
    Init(setup::InitArgs),
    /// Manage users in the auth store without starting the server.
    Admin(setup::AdminArgs),
}

/// How long the startup probe may spend validating storage backends.
///
/// Extracted so the fallback behaviour is pinned by tests rather than inferred
/// from a chain of combinators. A junk or zero value must land on the default,
/// not on zero — a 0s budget would time the probe out instantly and refuse to
/// start every server, turning a typo in an env var into a total outage.
/// Disabling the probe is `MEDIAGIT_STARTUP_PROBE=0`, a different knob.
fn startup_probe_timeout_secs(raw: Option<String>) -> u64 {
    const DEFAULT_SECS: u64 = 90;
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SECS)
}

#[cfg(test)]
mod startup_probe_tests {
    use super::startup_probe_timeout_secs;

    #[test]
    fn defaults_above_a_single_slow_backend_attempt() {
        // 30s was the old value and the bug: minio.rs configures
        // read_timeout(120s) with 2 attempts and no operation timeout, so the
        // probe used to give up on calls the SDK was still waiting on.
        // 20260824-ga16 (minio) and 20260824-ga14 (gcs) both died that way.
        assert_eq!(startup_probe_timeout_secs(None), 90);
        assert!(
            startup_probe_timeout_secs(None) > 30,
            "the default must exceed the 30s budget that produced the false failures"
        );
    }

    #[test]
    fn an_explicit_value_wins() {
        assert_eq!(startup_probe_timeout_secs(Some("150".into())), 150);
        assert_eq!(startup_probe_timeout_secs(Some(" 45 ".into())), 45);
    }

    #[test]
    fn junk_and_zero_fall_back_instead_of_disabling_startup() {
        // The load-bearing half. A 0 or unparseable value must NOT become a 0s
        // budget: that would time out instantly and refuse to start every
        // server, so one typo in an env var becomes a total outage.
        for raw in ["0", "", "abc", "-5", "12x"] {
            assert_eq!(
                startup_probe_timeout_secs(Some(raw.into())),
                90,
                "{raw:?} must fall back to the default, never to 0"
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // google-cloud-storage v1 enables aws-lc-rs by default; this crate already
    // depends on rustls with the `ring` feature for inbound TLS. With both
    // providers compiled in, rustls 0.23 refuses to pick automatically and
    // panics on first TLS use ("Could not automatically determine the
    // process-level CryptoProvider"). Install ring explicitly to match the
    // existing inbound TLS path. `_ =` swallows the "already installed" error
    // if some other entry point (e.g. test harness) raced us.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Parse CLI arguments
    let args = Args::parse();

    // Setup subcommands (init / admin) short-circuit before any of the
    // normal serve-path config loading below. No subcommand -> None -> falls
    // straight through to the existing bare-invocation / --config / --port /
    // --data-dir serve path, unchanged.
    if let Some(cmd) = args.command {
        return match cmd {
            Cmd::Init(init_args) => setup::run_init(&init_args).await,
            Cmd::Admin(admin_args) => setup::run_admin(&admin_args).await,
        };
    }

    // Setup tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "mediagit_server=debug,tower_http=debug,mediagit_storage=warn".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration from file (use path from --config, default is "mediagit-server.toml")
    let mut config = ServerConfig::load(&args.config)?;

    // Refuse to serve with at-rest encryption switched on but no usable master
    // key. Deferring this to the first escrow request would mean the operator
    // hears about it from a user whose push failed.
    mediagit_server::encryption::verify_startup_config(&config.encryption)?;
    let encryption_master = mediagit_server::encryption::load_master_key(&config.encryption)?;
    if config.encryption.enabled {
        tracing::info!(
            "At-rest encryption: enabled (repository keys wrapped under the server master key)"
        );
    }

    // Override config with CLI arguments if provided
    if let Some(port) = args.port {
        tracing::info!("Overriding port from CLI: {} -> {}", config.port, port);
        config.port = port;
    }
    if let Some(host) = args.host {
        tracing::info!("Overriding host from CLI: {} -> {}", config.host, host);
        config.host = host;
    }
    if let Some(data_dir) = args.data_dir {
        tracing::info!(
            "Overriding repos_dir from CLI: {:?} -> {:?}",
            config.repos_dir,
            data_dir
        );
        config.repos_dir = data_dir;
    }

    tracing::info!("Server configuration: {:?}", config);

    // Startup summary banner — operators should see at a glance what's wired.
    let bind_addr = if config.enable_tls {
        format!("{}:{} (TLS)", config.host, config.tls_port)
    } else {
        format!("{}:{} (HTTP)", config.host, config.port)
    };
    let auth_state = if config.enable_auth { "ON" } else { "OFF" };
    let rl_state = if config.enable_rate_limiting {
        format!(
            "ON ({} rps, burst {}, per credential)",
            config.rate_limit_rps, config.rate_limit_burst
        )
    } else {
        "OFF".to_string()
    };
    tracing::info!(
        "mediagit-server | listen={} | repos={} | auth={} | rate_limit={}",
        bind_addr,
        config.repos_dir.display(),
        auth_state,
        rl_state,
    );

    // P0-4: refuse to bind a non-loopback address with auth disabled — that's
    // an open server on the network with zero credentials. Loopback-only binds
    // are still allowed (matches today's local-dev default). Escape hatch for
    // operators who really want this: MEDIAGIT_ALLOW_INSECURE_BIND=1.
    if !config.enable_auth
        && !is_loopback_host(&config.host)
        && std::env::var("MEDIAGIT_ALLOW_INSECURE_BIND").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing to start: host '{}' is not loopback-only and `enable_auth` is false \
             in the server config (config keys: enable_auth, host). Either set `enable_auth = true` \
             (and configure `jwt_secret`), bind to 127.0.0.1/localhost, or set \
             MEDIAGIT_ALLOW_INSECURE_BIND=1 to override at your own risk.",
            config.host
        );
    }

    // Create repos directory if it doesn't exist
    std::fs::create_dir_all(&config.repos_dir)?;
    tracing::info!("Repositories directory: {:?}", config.repos_dir);

    // AU-10: one process per directory. Taken before any store is opened, so a
    // second instance is refused before it can load — and therefore before it
    // can later full-rewrite — the auth store, the lock file or the repo state.
    // See `instance_lock` for the eight pieces of shared state that a sibling
    // silently corrupts.
    //
    // Bound with `let _name`, not `let _`: `let _` would drop the guard on the
    // spot and release the lock while the server ran on unprotected.
    let _instance_lock =
        mediagit_server::instance_lock::acquire_or_warn(&config.repos_dir, &bind_addr)?;

    // The auth store defaults to a *sibling* of repos_dir (`../auth`), so
    // locking repos_dir alone leaves two servers with separate repo dirs free
    // to share — and shred — one auth store.
    let auth_dir = config.resolved_auth_store_dir();
    let _auth_lock = if config.enable_auth && auth_dir != config.repos_dir {
        mediagit_server::instance_lock::acquire_or_warn(&auth_dir, &bind_addr)?
    } else {
        None
    };

    // DC-4: build the registry *before* the state, so the same instance backs
    // both the recording side (handlers, via AppState) and the scrape endpoint.
    // It used to be constructed inside the metrics block below and moved
    // straight into the server, unreachable from any handler — which is why
    // `/metrics` served permanent zeros. `None` unless the endpoint is enabled,
    // so the default deployment pays one Option check per request.
    let metrics_registry: Option<mediagit_metrics::MetricsRegistry> =
        if std::env::var("MEDIAGIT_METRICS_ADDR").is_ok() {
            match mediagit_metrics::MetricsRegistry::new() {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::error!("Failed to create metrics registry: {}", e);
                    None
                }
            }
        } else {
            None
        };

    // Setup shared state with optional authentication
    let state = if config.enable_auth {
        // I2: MEDIAGIT_JWT_SECRET env var wins over the TOML `jwt_secret` key
        // (lets operators keep the secret out of the config file / repo).
        let env_jwt_secret = std::env::var("MEDIAGIT_JWT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        if env_jwt_secret.is_some() && config.jwt_secret.is_some() {
            tracing::warn!(
                "Both MEDIAGIT_JWT_SECRET env var and `jwt_secret` in the config file are set; \
                 using the env var"
            );
        }
        let jwt_secret = env_jwt_secret
            .as_deref()
            .or(config.jwt_secret.as_deref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "JWT secret is required when authentication is enabled \
                     (set `jwt_secret` in the config file or the MEDIAGIT_JWT_SECRET env var)"
                )
            })?;
        tracing::info!("Authentication is ENABLED");
        let auth_store_dir = config.resolved_auth_store_dir();
        let state = Arc::new(attach_metrics(
            AppState::new_with_full_auth(config.repos_dir.clone(), jwt_secret, &auth_store_dir)?
                .with_presigned_ttl(config.presigned_url_ttl_seconds)
                .with_verify_chunks_on_complete(config.verify_content_on_complete)
                .with_encryption_master(encryption_master.clone()),
            metrics_registry.clone(),
        ));

        // AU-3: warn when the defaults compose into "no tenant isolation".
        //
        // Three separate, individually-defensible backward-compat choices
        // combine badly: registration is open by default, self-registration
        // assigns Role::Write, and per-repo grant enforcement only switches on
        // once at least one grant exists (`check_permission`'s
        // `!grants.is_empty()`). On a freshly provisioned server with no grants
        // recorded, that means anyone who can reach POST /auth/register gets
        // write access to every repository.
        //
        // Not changed silently here: closing registration or defaulting to
        // Role::Read is a breaking change for existing deployments and for the
        // QA drills that self-register. Warning loudly is the honest
        // non-breaking move; the operator can then choose.
        // AU-3: a server with auth on and no admin cannot be administered.
        //
        // Self-registration deliberately creates Read-role accounts and there
        // is no client-controlled `role` field, so an admin can only come from
        // the offline `mediagit-server admin create` bootstrap. Starting
        // without one leaves nobody able to grant access, promote a user, or
        // manage keys — and no in-band way to fix it.
        if let Some(svc) = state.auth_service() {
            let admins = svc
                .credentials_store
                .list_users()
                .await
                .into_iter()
                .filter(|u| u.role == mediagit_security::auth::user::Role::Admin)
                .count();
            if admins == 0 {
                tracing::warn!(
                    "SECURITY: authentication is enabled but NO admin account exists. \
                     Self-registration creates read-only accounts, so nothing can grant \
                     access, promote users or manage API keys. Create one with: \
                     `mediagit-server admin create --username <name> --email <email>`"
                );
            }
        }

        if state.grants.is_empty() && config.allow_open_registration {
            tracing::warn!(
                "SECURITY: no per-repo grants are recorded, so per-repo authorization is \
                 INACTIVE and every authenticated user can read and write EVERY repository. \
                 Registration is also open, and self-registered users receive write \
                 permissions. Record at least one grant to activate per-repo enforcement, \
                 and/or set `allow_open_registration = false` in the server config."
            );
        }

        state
    } else {
        tracing::warn!("Authentication is DISABLED - not suitable for production!");
        Arc::new(attach_metrics(
            AppState::new(config.repos_dir.clone())
                .with_presigned_ttl(config.presigned_url_ttl_seconds)
                .with_verify_chunks_on_complete(config.verify_content_on_complete)
                .with_encryption_master(encryption_master.clone()),
            metrics_registry.clone(),
        ))
    };

    if config.verify_content_on_complete {
        tracing::info!(
            "Chunk content verification on complete is ENABLED (server reads back and \
             BLAKE3-verifies every presigned-uploaded chunk; set \
             verify_content_on_complete = false to disable)"
        );
    } else {
        tracing::warn!(
            "Chunk content verification on complete is DISABLED - presigned uploads are only \
             checked for existence, not content; a client with repo:write can store bytes that \
             do not match their claimed chunk id"
        );
    }

    // I1: startup probe — validate every repo's storage backend construction
    // before we start accepting traffic, so a bad S3/Azure/GCS config surfaces
    // as a boot failure instead of a 500 on a client's first request.
    // MEDIAGIT_STARTUP_PROBE=0 disables it.
    if std::env::var("MEDIAGIT_STARTUP_PROBE").as_deref() != Ok("0") {
        let repo_dirs: Vec<PathBuf> = std::fs::read_dir(&config.repos_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();

        if repo_dirs.is_empty() {
            tracing::debug!(
                "Startup probe: no repos found under {:?}, skipping",
                config.repos_dir
            );
        } else {
            let repo_count = repo_dirs.len();
            tracing::info!(
                "Startup probe: validating storage backend for {} repo(s)...",
                repo_count
            );
            let probe_state = Arc::clone(&state);
            let probe = async move {
                use futures::stream::{self, StreamExt};
                stream::iter(repo_dirs.into_iter().map(|repo_path| {
                    let probe_state = Arc::clone(&probe_state);
                    async move {
                        // Log each repo as it FINISHES. The outer timeout kills
                        // the whole future at once, so without this a timeout
                        // says nothing about which repo was stuck - and that is
                        // the one fact the diagnosis starts from. With it, the
                        // repos that completed are named and the missing one is
                        // the suspect.
                        let started = std::time::Instant::now();
                        let result = mediagit_server::handlers::get_or_init_storage(
                            &probe_state,
                            &repo_path,
                        )
                        .await;
                        tracing::info!(
                            repo = ?repo_path,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            ok = result.is_ok(),
                            "Startup probe: repo validated"
                        );
                        (repo_path, result)
                    }
                }))
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await
            };

            // 30s was arbitrary, and SHORTER than the patience of the very
            // backend it probes. `minio.rs` configures read_timeout(120s) with
            // retry max_attempts=2 and deliberately sets no operation timeout,
            // so a single head_bucket can legitimately outlast a 30s budget —
            // the probe then kills a call the SDK was still waiting on and
            // reports a bare timeout with no cause.
            //
            // Measured twice, on two different backends:
            //   20260824-ga16  minio/Silo  "startup probe timed out after 30s"
            //                  with SEVEN 07_abuse servers live against the same
            //                  bucket; Silo answered health in 69ms right after.
            //   20260824-ga14  gcs         same bare 30s timeout, no error
            // (ga14's AWS failure was genuinely different - explicit
            //  "dispatch failure: io error" - i.e. a real outage, not this.)
            //
            // 90s is chosen to exceed one full attempt against a slow-but-alive
            // backend without approaching read_timeout x max_attempts (240s):
            // the probe's job is to fail fast on a MISCONFIGURED backend (wrong
            // creds, missing bucket), not to double as a load test.
            // MEDIAGIT_STARTUP_PROBE_TIMEOUT_SECS tunes it.
            let probe_timeout_secs = startup_probe_timeout_secs(
                std::env::var("MEDIAGIT_STARTUP_PROBE_TIMEOUT_SECS").ok(),
            );
            let probe_started = std::time::Instant::now();
            let results =
                tokio::time::timeout(std::time::Duration::from_secs(probe_timeout_secs), probe)
                    .await
                    .map_err(|_| {
                        // Say what it was waiting on. The old message named neither the
                        // repo nor the backend, so a timeout left a silent hole exactly
                        // where the diagnosis needed to start - the same failure that
                        // made ga14's GCS row look like a network outage when it may
                        // have been this.
                        anyhow::anyhow!(
                            "startup probe timed out after {}s validating {} repo(s) \
                     (waited {:.1}s; backend read_timeout is 120s with 2 attempts, \
                     so a slow-but-alive backend can outlast this budget). \
                     Raise MEDIAGIT_STARTUP_PROBE_TIMEOUT_SECS, or set \
                     MEDIAGIT_STARTUP_PROBE=0 to skip this check",
                            probe_timeout_secs,
                            repo_count,
                            probe_started.elapsed().as_secs_f64()
                        )
                    })?;

            let mut failed = 0;
            for (repo_path, result) in results {
                if let Err(status) = result {
                    failed += 1;
                    tracing::error!(
                        "Startup probe failed for repo {:?}: storage backend init returned {}",
                        repo_path,
                        status
                    );
                }
            }
            if failed > 0 {
                anyhow::bail!(
                    "startup probe failed for {} of {} repo(s); see errors above for details, \
                     or set MEDIAGIT_STARTUP_PROBE=0 to skip this check",
                    failed,
                    repo_count
                );
            }
            tracing::info!("Startup probe passed for {} repo(s)", repo_count);
        }
    }

    // Durability sweep: resume any pack verification an earlier crash
    // interrupted. Every `.mediagit/packs/<shard>/<pack_oid>.pending` marker
    // is an orphan left by `complete_pack` (see
    // `mediagit_server::handlers::repo::verify_pack_in_background`) — the
    // marker is the durable source of truth for "unverified", so finding one
    // here means the in-memory `unverified_packs` set from the crashed
    // process is gone but the pack itself never got a second look.
    //
    // Deliberately unconditional (not gated by MEDIAGIT_STARTUP_PROBE): the
    // probe above validates *configuration*, this repairs *data safety*.
    // `resume_pack_verification` itself never bails — an unreadable marker or
    // manifest is logged and the pack is simply left unverified for a future
    // sweep, never a boot failure. Runs before the listener accepts traffic,
    // but does not block on verification finishing — it re-enqueues the same
    // background worker `complete_pack` uses and returns; the read path
    // refuses to serve an unverified pack blind in the meantime.
    {
        let sweep_repo_dirs: Vec<PathBuf> = std::fs::read_dir(&config.repos_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();

        let mut resumed = 0usize;
        for repo_path in sweep_repo_dirs {
            let Some(repo_name) = repo_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let packs_dir = repo_path.join(".mediagit").join("packs");
            let Ok(mut shard_dirs) = tokio::fs::read_dir(&packs_dir).await else {
                continue;
            };
            while let Ok(Some(shard)) = shard_dirs.next_entry().await {
                let shard_path = shard.path();
                if !shard_path.is_dir() {
                    continue;
                }
                let Ok(mut files) = tokio::fs::read_dir(&shard_path).await else {
                    continue;
                };
                while let Ok(Some(file)) = files.next_entry().await {
                    let file_path = file.path();
                    if file_path.extension().and_then(|e| e.to_str()) != Some("pending") {
                        continue;
                    }
                    let Some(pack_oid) = file_path.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    tracing::warn!(
                        repo = repo_name,
                        pack = pack_oid,
                        "Startup sweep: found orphaned .pending marker; resuming verification"
                    );
                    mediagit_server::handlers::resume_pack_verification(
                        &state, &repo_path, repo_name, pack_oid,
                    )
                    .await;
                    resumed += 1;
                }
            }
        }
        if resumed > 0 {
            tracing::info!(
                resumed,
                "Startup sweep: resumed verification for orphaned pending pack(s)"
            );
        }
    }

    // P1-1: optional Prometheus /metrics endpoint on a separate listener, off
    // by default. Set MEDIAGIT_METRICS_ADDR=host:port to enable (e.g.
    // 127.0.0.1:9090). Pure wiring of the existing mediagit-metrics crate —
    // no metric-recording calls are added to request handlers here.
    if let Ok(metrics_addr) = std::env::var("MEDIAGIT_METRICS_ADDR") {
        match metrics_addr.rsplit_once(':') {
            Some((bind_address, port_str)) if port_str.parse::<u16>().is_ok() => {
                let port: u16 = port_str.parse().expect("checked above");
                let bind_address = bind_address.to_string();
                // The *same* registry the handlers record into. Constructing
                // a second one here is what made this endpoint a liar.
                match metrics_registry.clone() {
                    Some(registry) => {
                        let metrics_config = mediagit_metrics::MetricsConfig {
                            port,
                            enabled: true,
                            bind_address,
                        };
                        tracing::info!("Metrics endpoint ENABLED on {}", metrics_addr);
                        let server =
                            mediagit_metrics::MetricsServer::with_config(registry, metrics_config);
                        tokio::spawn(async move {
                            if let Err(e) = server.serve().await {
                                tracing::error!("Metrics server error: {}", e);
                            }
                        });
                    }
                    None => {
                        tracing::error!(
                            "Metrics endpoint requested but the registry failed to build; \
                                /metrics not started"
                        );
                    }
                }
            }
            _ => {
                tracing::error!(
                    "MEDIAGIT_METRICS_ADDR='{}' is not a valid host:port address; metrics endpoint not started",
                    metrics_addr
                );
            }
        }
    }

    // Build router with optional rate limiting
    let (app, rate_limiter) = if config.enable_rate_limiting {
        tracing::info!(
            "Rate limiting ENABLED: {} req/s, burst {}, keyed per credential (client IP for unauthenticated requests)",
            config.rate_limit_rps,
            config.rate_limit_burst
        );
        let rate_config = RateLimitConfig {
            requests_per_second: config.rate_limit_rps,
            burst_size: config.rate_limit_burst,
        };
        let (router, cleanup, limiter) =
            create_rate_limited_router(Arc::clone(&state), rate_config);

        // Spawn rate limiter cleanup task
        std::thread::spawn(cleanup);

        (router, Some(limiter))
    } else {
        tracing::warn!("Rate limiting is DISABLED - not suitable for production!");
        (create_router(Arc::clone(&state)), None)
    };
    // I4: CORS is off unless `cors_allowed_origins` is set in config.
    let app = mediagit_server::apply_cors_layer(app, config.cors_allowed_origins.as_deref());

    // Start HTTP server (always enabled)
    let http_bind_addr = config.bind_addr();
    tracing::info!("Starting HTTP server on {}", http_bind_addr);

    // If TLS is enabled, start both HTTP and HTTPS servers concurrently.
    // I5: the two `#[cfg(...)]` blocks below (rather than gating the whole
    // if/else on `#[cfg(feature = "tls")]`) exist so a non-tls build with
    // `enable_tls = true` fails loudly at boot instead of silently falling
    // through — previously the entire if/else (HTTP-only branch included)
    // vanished under `#[cfg(feature = "tls")]`, so a non-tls build never
    // bound anything.
    if config.enable_tls {
        #[cfg(not(feature = "tls"))]
        {
            anyhow::bail!(
                "enable_tls=true in configuration but this binary was built without the `tls` \
                 feature; refusing to silently fall back to plain HTTP. Rebuild with \
                 `--features tls` or set `enable_tls = false`."
            );
        }

        #[cfg(feature = "tls")]
        {
            let https_bind_addr = config.tls_bind_addr();
            tracing::info!("Starting HTTPS server on {}", https_bind_addr);

            // Build TLS configuration
            let tls_config = config.build_tls_config()?;
            let certificate = tls_config.load_certificate()?;

            // Build axum-server RustlsConfig from certificate
            let rustls_config = build_axum_rustls_config(&certificate, tls_config.min_tls_version)?;

            // SV-1: the HTTPS listener must carry the same rate limiter as
            // HTTP. It used to call `create_router` unconditionally, so
            // enabling TLS silently disabled rate limiting on the port most
            // likely to face the internet — while startup still logged
            // "Rate limiting ENABLED". The limiter is *shared*, not rebuilt:
            // two independent governors would give an attacker twice the
            // budget simply for splitting traffic across the two ports.
            let https_app = match &rate_limiter {
                Some(limiter) => {
                    create_router_sharing_rate_limit(Arc::clone(&state), Arc::clone(limiter))
                }
                None => create_router(Arc::clone(&state)),
            };
            let https_app = mediagit_server::apply_cors_layer(
                https_app,
                config.cors_allowed_origins.as_deref(),
            );

            // Run both servers concurrently.
            //
            // Bind HTTP before announcing it, for the reason spelled out in the
            // HTTP-only branch below: the announcement must be an observation,
            // not a promise. HTTPS is bound inside `axum_server::bind_rustls`,
            // so its bind failure surfaces through the `select!` below instead;
            // the message says "binding" for that half rather than claiming a
            // listener that may not exist yet.
            let http_listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
            let http_local = http_listener.local_addr()?;
            tracing::info!(
                "MediaGit server listening on HTTP: {} | HTTPS binding on: {}",
                http_local,
                https_bind_addr
            );
            tracing::info!("Press Ctrl+C to stop");

            // Spawn HTTP server task
            let http_server = tokio::spawn(async move {
                // ConnectInfo must be supplied or SmartIpKeyExtractor (rate limiting)
                // 500s with "Unable to extract key!" on every request.
                axum::serve(
                    http_listener,
                    app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(shutdown_signal())
                .await
            });

            // Spawn HTTPS server task
            let https_handle = axum_server::Handle::new();
            let https_shutdown_handle = https_handle.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                https_shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
            });
            let https_server = tokio::spawn(async move {
                let addr: std::net::SocketAddr = https_bind_addr
                    .parse()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
                axum_server::bind_rustls(addr, rustls_config)
                    .handle(https_handle)
                    // ConnectInfo, for the same reason the two HTTP listeners
                    // supply it: without it the rate limiter's IP fallback
                    // 500s with "Unable to extract key!" on every request.
                    // This listener was the one that did not, so TLS plus
                    // `enable_rate_limiting = true` failed every HTTPS request
                    // -- on the port the SV-1 note above calls the one most
                    // likely to face the internet. Invisible because rate
                    // limiting is off by default and no test enabled it.
                    .serve(https_app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                    .await
            });

            // Wait for both servers (or either to fail)
            tokio::select! {
                result = http_server => {
                    result??;
                }
                result = https_server => {
                    result??;
                }
            }
        }
    } else {
        // HTTP only mode
        //
        // Bind BEFORE announcing readiness. These two lines used to sit above
        // the bind, which made "MediaGit server listening" a promise rather
        // than an observation: a bind that failed with EADDRINUSE, or a process
        // that never reached `accept()`, produced a startup log byte-identical
        // to a healthy one. That cost real diagnosis time - the ga15/ga18
        // campaign wedges could not be told apart from "bound but not serving"
        // after the fact, because the only surviving evidence was a log that
        // claimed success either way.
        let listener = tokio::net::TcpListener::bind(&http_bind_addr).await?;
        // Report the address the kernel actually handed us, not the one we
        // asked for: they differ whenever the configured port is 0.
        let local_addr = listener.local_addr()?;
        tracing::info!("MediaGit server listening on {}", local_addr);
        tracing::info!("Press Ctrl+C to stop");

        // ConnectInfo must be supplied or SmartIpKeyExtractor (rate limiting)
        // 500s with "Unable to extract key!" on every request.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    }

    Ok(())
}

/// Check whether `host` only ever resolves to the local machine (loopback).
/// Used to gate the P0-4 insecure-bind refusal: a loopback bind with auth
/// disabled is still local-only and safe for dev; anything else (0.0.0.0, a
/// real interface IP, or a hostname) with auth off is an open server.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Resolves when the process receives Ctrl+C (all platforms) or SIGTERM
/// (unix only). Used to drive `.with_graceful_shutdown(...)` so in-flight
/// requests (e.g. a multi-GB chunk upload) finish instead of being hard-cut.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, starting graceful shutdown"),
        _ = terminate => tracing::info!("Received SIGTERM, starting graceful shutdown"),
    }
}

/// Map a configured TLS floor onto the exact protocol-version list handed to rustls.
///
/// Extracted from `build_axum_rustls_config` purely so it is assertable:
/// that function needs a parsed certificate and returns an opaque
/// `RustlsConfig` whose accepted versions cannot be read back, so the one
/// thing worth checking — that `tls_min_version` actually reaches rustls —
/// was untestable inline. Same reason `clamp_cap`, `case_only_collisions`
/// and `ensure_writable_object_size` are separate functions in this repo.
///
/// A *minimum* of 1.2 means 1.2 **and** 1.3 are accepted; it is not "1.2 only".
#[cfg(feature = "tls")]
fn rustls_protocol_versions(
    min_version: mediagit_security::TlsVersion,
) -> &'static [&'static rustls::SupportedProtocolVersion] {
    // `static`, not an inline `&[..]` literal: temporary lifetime extension
    // applies to a `let` binding (which is why this compiled inline) but not
    // to a returned reference — E0515.
    static V1_3_ONLY: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];
    static V1_2_AND_UP: &[&rustls::SupportedProtocolVersion] =
        &[&rustls::version::TLS12, &rustls::version::TLS13];

    match min_version {
        mediagit_security::TlsVersion::V1_3 => V1_3_ONLY,
        mediagit_security::TlsVersion::V1_2 => V1_2_AND_UP,
    }
}

/// Build axum-server RustlsConfig from Certificate
#[cfg(feature = "tls")]
fn build_axum_rustls_config(
    certificate: &mediagit_security::Certificate,
    min_version: mediagit_security::TlsVersion,
) -> Result<axum_server::tls_rustls::RustlsConfig> {
    use axum_server::tls_rustls::RustlsConfig;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    // Parse certificate PEM
    let cert_pem = certificate.cert_pem.as_bytes();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate: {}", e))?;

    // Parse private key PEM (handles PKCS#8, RSA PKCS#1, and EC SEC1 automatically)
    let key_pem = certificate.key_pem.as_bytes();
    let private_key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?;

    // DC-9: honour `min_tls_version` instead of taking rustls's defaults.
    //
    // `TlsConfig::min_tls_version` defaults to 1.3 and four docs promised "TLS
    // 1.3 for all network operations", but this function built a fresh
    // `ServerConfig::builder()` and never consulted the setting — and rustls's
    // default accepts **1.2 as well**. An operator reading the config believed
    // 1.3-only while the server happily negotiated 1.2: a silent downgrade,
    // which is worse than an honest 1.2 because nobody goes looking.
    let versions = rustls_protocol_versions(min_version);
    tracing::info!("TLS minimum version: {:?}", min_version);

    // Build rustls ServerConfig with ALPN to enable HTTP/2 negotiation.
    //
    // `with_no_client_auth` is deliberate and load-bearing: mTLS
    // (`TlsConfig::client_ca_path` / `require_client_cert`) is **not wired**,
    // and there is no operator-facing knob for it in `mediagit-server.toml`.
    // Reading those fields here without a way to set them would be theatre.
    let mut rustls_config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_single_cert(certs, private_key)
        .map_err(|e| anyhow::anyhow!("Failed to build TLS config: {}", e))?;
    rustls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    // Convert to axum-server RustlsConfig
    Ok(RustlsConfig::from_config(Arc::new(rustls_config)))
}

/// Attach the metrics registry to state when `/metrics` is enabled.
///
/// A free function rather than inlining `.with_metrics()` at both construction
/// sites: `with_metrics` takes a registry, not an `Option`, so each call site
/// would otherwise need its own `if let` around an already-long expression.
fn attach_metrics(
    state: mediagit_server::AppState,
    registry: Option<mediagit_metrics::MetricsRegistry>,
) -> mediagit_server::AppState {
    match registry {
        Some(r) => state.with_metrics(r),
        None => state,
    }
}

#[cfg(all(test, feature = "tls"))]
mod tls_version_tests {
    use super::rustls_protocol_versions;
    use mediagit_security::TlsVersion;

    /// The default. Nothing below 1.3 may be offered.
    #[test]
    fn v1_3_floor_offers_only_tls13() {
        let v = rustls_protocol_versions(TlsVersion::V1_3);
        assert_eq!(v.len(), 1, "1.3 floor must offer exactly one version");
        assert_eq!(v[0].version, rustls::ProtocolVersion::TLSv1_3);
    }

    /// The escape hatch. A *minimum* of 1.2 must still allow 1.3 — an
    /// operator relaxing the floor for one legacy client must not thereby
    /// downgrade every modern client to 1.2.
    #[test]
    fn v1_2_floor_offers_both_and_still_allows_tls13() {
        let v = rustls_protocol_versions(TlsVersion::V1_2);
        let got: Vec<_> = v.iter().map(|p| p.version).collect();
        assert!(
            got.contains(&rustls::ProtocolVersion::TLSv1_2),
            "1.2 floor must accept 1.2: {got:?}"
        );
        assert!(
            got.contains(&rustls::ProtocolVersion::TLSv1_3),
            "relaxing the floor must not disable 1.3: {got:?}"
        );
    }

    /// Pins the asymmetry the two tests above rely on: the floors must not
    /// resolve to the same list, or the knob would be inert while both
    /// tests still passed.
    #[test]
    fn the_two_floors_are_not_the_same_list() {
        let a: Vec<_> = rustls_protocol_versions(TlsVersion::V1_3)
            .iter()
            .map(|p| p.version)
            .collect();
        let b: Vec<_> = rustls_protocol_versions(TlsVersion::V1_2)
            .iter()
            .map(|p| p.version)
            .collect();
        assert_ne!(a, b, "tls_min_version has no effect on what rustls offers");
    }
}
