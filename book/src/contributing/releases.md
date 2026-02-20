# Release Process

The MediaGit release process for maintainers. Releases are driven by GitHub Actions.

## Version Numbering

Semantic versioning: `MAJOR.MINOR.PATCH[-prerelease]`

- `v0.1.0` — stable release
- `v0.2.0-alpha.1` — pre-release (alpha/beta/rc in version → `is-prerelease: true`)

## Pre-Release Checklist

Before creating a release tag:

```bash
# 1. Ensure all CI jobs are green on main
# 2. Update CHANGELOG.md with the new version entry
# 3. Bump version in workspace Cargo.toml [workspace.package]
#    (all crates inherit version.workspace = true)
sed -i 's/^version = ".*"/version = "0.2.0"/' Cargo.toml

# 4. Update Cargo.lock
cargo generate-lockfile

# 5. Verify the build
cargo build --release --bin mediagit --bin mediagit-server

# 6. Run full test suite
cargo test --workspace --all-features

# 7. Check MSRV still passes
cargo +1.91.0 check --workspace --all-features

# 8. Commit and push
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: release v0.2.0"
git push origin main
```

## Creating the Release Tag

```bash
# Stable release
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0

# Pre-release (alpha/beta/rc)
git tag -a v0.2.0-alpha.1 -m "Pre-release v0.2.0-alpha.1"
git push origin v0.2.0-alpha.1
```

Pushing the tag automatically triggers the `release.yml` workflow.

## Release Workflow Steps

The `release.yml` workflow runs these jobs in sequence:

### 1. plan
Determines the version from the tag, sets `is-prerelease` flag.

### 2. build (parallel, 5 targets)

| Target | Platform | Tool |
|--------|----------|------|
| `x86_64-unknown-linux-gnu` | Linux x64 | `cargo` |
| `aarch64-unknown-linux-gnu` | Linux ARM64 | `cross` |
| `x86_64-apple-darwin` | macOS Intel | `cargo` |
| `aarch64-apple-darwin` | macOS Apple Silicon | `cargo` |
| `x86_64-pc-windows-msvc` | Windows x64 | `cargo` |

Both `mediagit` and `mediagit-server` binaries are built and archived.

> **Note**: Windows ARM64 (`aarch64-pc-windows-msvc`) is not released — `cross-rs` does not support Windows targets. Windows ARM64 users should build from source.

### 3. installers
Generates `install.sh` (Unix) and `install.ps1` (Windows) scripts.

### 4. release
Creates the GitHub Release with all archives, checksums, and installer scripts.
Only runs on tag push (not `workflow_dispatch`).

### 5. publish-crates
Publishes all 13 crates to crates.io in dependency order. Only runs for stable releases (`is-prerelease == false`).

**Publish order** (respects internal dependency tiers):
1. Tier 0: `mediagit-config`, `mediagit-security`, `mediagit-observability`, `mediagit-compression`, `mediagit-storage`, `mediagit-media`, `mediagit-git`
2. Tier 1: `mediagit-versioning`, `mediagit-metrics`, `mediagit-migration`
3. Tier 2: `mediagit-protocol`
4. Tier 3: `mediagit-server`, `mediagit-cli`

The publish step uses retry logic (3 attempts, 60s between) and 30s delays between crates for crates.io index propagation.

### 6. docker
Builds and pushes multi-arch Docker images to GHCR (`ghcr.io/mediagit/mediagit-core`).
Tags: `latest` (stable only), semver `X.Y.Z`, `X.Y`, `X`.

## Required Secrets

| Secret | Value |
|--------|-------|
| `CARGO_REGISTRY_TOKEN` | crates.io API token with publish scope |
| `GITHUB_TOKEN` | Auto-provided by GitHub Actions |

## Dry Run

Test the release workflow without publishing:

1. Go to **Actions → Release → Run workflow**
2. Check **Dry run** checkbox
3. Click **Run workflow**

The dry run builds all binaries and creates a pre-release with tag `dry-run`. No crates are published and no Docker images are pushed.

## Post-Release

After a successful release:

1. Verify [GitHub Releases](https://github.com/mediagit/mediagit-core/releases) has all assets
2. Verify crates on [crates.io](https://crates.io/crates/mediagit-cli)
3. Verify Docker image: `docker pull ghcr.io/mediagit/mediagit-core:latest`
4. Update the [documentation site](https://docs.mediagit.dev) if needed
5. Announce on Discord/community channels

## Hotfix Releases

For critical bug fixes on a stable release:

```bash
# Create hotfix branch from the tag
git checkout -b hotfix/v0.1.1 v0.1.0

# Apply the fix, test, commit
# ...

# Tag and push
git tag -a v0.1.1 -m "Hotfix v0.1.1: fix critical bug"
git push origin v0.1.1

# Merge fix back to main
git checkout main
git merge hotfix/v0.1.1
git push origin main
```
