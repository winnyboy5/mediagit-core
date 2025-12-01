# Release Process

This document describes the release process for MediaGit.

## Overview

MediaGit uses an automated release process powered by GitHub Actions. Releases are triggered by pushing git tags and automatically:

1. Build binaries for all 6 platforms (Linux/macOS/Windows on x86_64/ARM64)
2. Generate installation scripts (shell, PowerShell)
3. Create GitHub Release with all artifacts
4. Publish to crates.io
5. Build and publish Docker images to GitHub Container Registry

## Prerequisites

1. **Permissions**: Write access to the repository
2. **Secrets**: The following secrets must be configured in GitHub:
   - `CARGO_REGISTRY_TOKEN`: Token for publishing to crates.io
   - `GITHUB_TOKEN`: Automatically provided by GitHub Actions

## Release Checklist

### 1. Prepare the Release

- [ ] Update version in `Cargo.toml` (workspace.package.version)
- [ ] Update version in all crate `Cargo.toml` files
- [ ] Update `CHANGELOG.md` with release notes
- [ ] Run tests: `cargo test --all-features`
- [ ] Run benchmarks: `cargo bench`
- [ ] Run security audit: `cargo audit`
- [ ] Update documentation if needed

### 2. Create Release Commit

```bash
# Update version to 0.2.0 (example)
vim Cargo.toml # Update [workspace.package] version

# Update CHANGELOG.md
vim CHANGELOG.md

# Commit changes
git add Cargo.toml CHANGELOG.md
git commit -m "chore: prepare release 0.2.0"
git push origin main
```

### 3. Create and Push Release Tag

```bash
# Create annotated tag
git tag -a v0.2.0 -m "Release version 0.2.0"

# Push tag to trigger release workflow
git push origin v0.2.0
```

### 4. Monitor Release Workflow

1. Go to [GitHub Actions](https://github.com/yourusername/mediagit-core/actions)
2. Watch the "Release" workflow run
3. Verify all jobs complete successfully:
   - plan
   - build (all 6 platforms)
   - installers
   - release
   - publish-crates
   - docker

### 5. Verify Release

After the workflow completes:

- [ ] Check GitHub Release page for all artifacts
- [ ] Verify checksums for all binaries
- [ ] Test installation scripts:
  ```bash
  # Unix/macOS
  curl -fsSL https://raw.githubusercontent.com/yourusername/mediagit-core/main/install.sh | sh

  # Windows PowerShell
  iwr https://raw.githubusercontent.com/yourusername/mediagit-core/main/install.ps1 | iex
  ```
- [ ] Verify crates.io publication: https://crates.io/crates/mediagit-cli
- [ ] Test Docker image:
  ```bash
  docker run --rm ghcr.io/yourusername/mediagit-core:0.2.0 --version
  ```

### 6. Post-Release Tasks

- [ ] Announce release on social media/blog
- [ ] Update documentation website
- [ ] Monitor for issues
- [ ] Update Homebrew tap (if separate repository)
- [ ] Submit to package managers:
  - [ ] Chocolatey Community Repository
  - [ ] AUR (Arch User Repository)
  - [ ] Debian/Ubuntu PPA (if needed)

## Versioning

MediaGit follows [Semantic Versioning](https://semver.org/):

- **MAJOR**: Incompatible API changes
- **MINOR**: New functionality (backwards compatible)
- **PATCH**: Bug fixes (backwards compatible)

### Pre-release Versions

For alpha/beta/rc releases:

```bash
# Alpha
git tag -a v0.2.0-alpha.1 -m "Release 0.2.0-alpha.1"

# Beta
git tag -a v0.2.0-beta.1 -m "Release 0.2.0-beta.1"

# Release Candidate
git tag -a v0.2.0-rc.1 -m "Release 0.2.0-rc.1"
```

Pre-release versions are automatically marked as "Pre-release" on GitHub.

## Hotfix Releases

For urgent bug fixes:

1. Create hotfix branch from the release tag:
   ```bash
   git checkout -b hotfix/0.2.1 v0.2.0
   ```

2. Make and commit the fix
3. Update version to patch release (0.2.1)
4. Create release tag
5. Cherry-pick fix back to main

## Troubleshooting

### Release Workflow Fails

1. Check the GitHub Actions logs for specific errors
2. Common issues:
   - **Build failure**: Check compilation errors in build logs
   - **Test failure**: Fix tests and re-tag
   - **Cargo publish error**: Check crates.io for duplicate version
   - **Docker build error**: Verify Dockerfile and binary paths

### Re-releasing

If you need to re-release (NOT RECOMMENDED):

1. Delete the GitHub Release
2. Delete the git tag locally and remotely:
   ```bash
   git tag -d v0.2.0
   git push origin :refs/tags/v0.2.0
   ```
3. If published to crates.io, you CANNOT unpublish. Must use a new version.
4. Fix issues and create a new tag

## Manual Release (Emergency)

If automated release fails completely:

1. Build locally for all platforms:
   ```bash
   cargo build --release --target x86_64-unknown-linux-gnu
   # ... repeat for other targets
   ```

2. Create archives manually
3. Create GitHub Release manually
4. Upload artifacts
5. Publish to crates.io manually:
   ```bash
   cargo publish -p mediagit-cli
   ```

## Package Manager Submissions

### Homebrew

Formula is auto-generated in `packaging/homebrew/mediagit.rb`. For Homebrew core:

1. Fork homebrew-core
2. Update formula
3. Submit PR to Homebrew/homebrew-core

### Chocolatey

Package configuration in `packaging/chocolatey/`:

1. Update version in `mediagit.nuspec`
2. Update checksums in `tools/chocolateyinstall.ps1`
3. Test locally: `choco pack`
4. Submit to Chocolatey Community: `choco push mediagit.0.2.0.nupkg --source https://push.chocolatey.org/`

### APT (Debian/Ubuntu)

For PPA distribution:

1. Build .deb packages using `packaging/apt/build-deb.sh`
2. Sign with GPG key
3. Upload to PPA
4. Update repository index

## Security

- All release assets include SHA-256 checksums
- Docker images are signed and include attestations
- Binaries are built in isolated GitHub Actions runners
- No credentials are embedded in binaries

## Support

For questions about the release process:
- Open an issue: https://github.com/yourusername/mediagit-core/issues
- Email: hello@mediagit.dev
