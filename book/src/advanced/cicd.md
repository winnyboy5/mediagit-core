# CI/CD Integration

Using MediaGit in continuous integration and deployment pipelines.

## Overview

MediaGit's CI/CD integration enables automated testing, verification, and deployment of media asset repositories. The `mediagit-server` binary provides the HTTP API for remote operations, while standard CLI commands work in headless CI environments.

## GitHub Actions

### Basic CI Workflow

```yaml
name: MediaGit CI

on: [push, pull_request]

jobs:
  verify-assets:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install MediaGit
        run: |
          curl -fsSL https://github.com/mediagit/mediagit-core/releases/latest/download/mediagit-x86_64-linux.tar.gz \
            | tar xz -C /usr/local/bin/

      - name: Verify repository integrity
        run: mediagit fsck

      - name: Check repository stats
        run: mediagit stats
```

### Full Pipeline with S3 Backend

```yaml
name: Media Asset Pipeline

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  AWS_REGION: us-east-1

jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install MediaGit
        run: |
          VERSION="0.1.0"
          curl -fsSL "https://github.com/mediagit/mediagit-core/releases/download/v${VERSION}/mediagit-${VERSION}-x86_64-linux.tar.gz" \
            | tar xz -C /usr/local/bin/

      - name: Configure author identity
        run: |
          cat >> .mediagit/config.toml << 'EOF'
          [author]
          name = "CI Bot"
          email = "ci@yourorg.com"
          EOF

      - name: Configure S3 backend
        run: |
          cat >> .mediagit/config.toml << 'EOF'
          [storage]
          backend = "s3"
          bucket = "${{ vars.MEDIAGIT_S3_BUCKET }}"
          region = "us-east-1"
          EOF
        env:
          AWS_ACCESS_KEY_ID: ${{ secrets.AWS_ACCESS_KEY_ID }}
          AWS_SECRET_ACCESS_KEY: ${{ secrets.AWS_SECRET_ACCESS_KEY }}

      - name: Verify assets
        run: mediagit verify

      - name: Run fsck
        run: mediagit fsck

      - name: Show stats
        run: mediagit stats
```

## Environment Variables

All CI systems can configure MediaGit through environment variables without modifying `config.toml`:

| Variable | Purpose |
|----------|---------|
| `MEDIAGIT_REPO` | Override repository path (used by `-C` flag) |
| `MEDIAGIT_AUTHOR_NAME` | Commit author name |
| `MEDIAGIT_AUTHOR_EMAIL` | Commit author email |
| `AWS_ACCESS_KEY_ID` | S3 access key |
| `AWS_SECRET_ACCESS_KEY` | S3 secret key |
| `AWS_REGION` | S3 region |
| `AWS_ENDPOINT_URL` | Custom S3 endpoint (MinIO, etc.) |
| `AZURE_STORAGE_CONNECTION_STRING` | Azure Blob connection string |
| `GCS_EMULATOR_HOST` | GCS emulator URL (for testing) |

See [Environment Variables Reference](../reference/environment.md) for the full list.

## Non-Interactive Operation

MediaGit CLI commands exit cleanly in non-interactive environments. Key flags:

```bash
# Specify repository explicitly (no directory navigation needed)
mediagit -C /path/to/repo status

# Commit with author from environment (no config needed)
MEDIAGIT_AUTHOR_NAME="CI Bot" \
MEDIAGIT_AUTHOR_EMAIL="ci@example.com" \
  mediagit commit -m "Automated update"

# Add all changed files
mediagit add --all

# Parallel add with explicit job count
mediagit add --jobs 8 assets/
```

## Integration with Storage Emulators

For running integration tests locally or in CI without cloud credentials, use the bundled Docker Compose setup:

```bash
# Start emulators (MinIO, Azurite, fake-gcs-server)
docker compose -f docker-compose.test.yml up -d

# Configure MediaGit to use MinIO
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_ENDPOINT_URL=http://localhost:9000
export AWS_REGION=us-east-1

# Run your pipeline
mediagit init test-repo
cd test-repo
mediagit add assets/
mediagit commit -m "Test commit"
mediagit push origin main

# Cleanup
docker compose -f docker-compose.test.yml down -v
```

## GitLab CI

```yaml
stages:
  - validate

validate-assets:
  stage: validate
  image: ubuntu:22.04
  before_script:
    - apt-get update -qq && apt-get install -y -qq curl
    - curl -fsSL https://github.com/mediagit/mediagit-core/releases/latest/download/mediagit-x86_64-linux.tar.gz
        | tar xz -C /usr/local/bin/
  script:
    - mediagit fsck
    - mediagit verify
    - mediagit stats
  variables:
    MEDIAGIT_AUTHOR_NAME: "GitLab CI"
    MEDIAGIT_AUTHOR_EMAIL: "ci@gitlab.com"
```

## Performance in CI

MediaGit is designed for CI performance:

- **Parallel add**: Use `-j N` to match your runner's CPU count
- **Cache**: The `.mediagit/` directory can be cached between CI runs for faster operations
- **Shallow operations**: `mediagit log -1` for recent commit checks is instantaneous

### Caching the MediaGit binary (GitHub Actions)

```yaml
- name: Cache MediaGit binary
  uses: actions/cache@v4
  with:
    path: /usr/local/bin/mediagit
    key: mediagit-${{ runner.os }}-0.1.0
```

## Troubleshooting CI Issues

### `mediagit: command not found`
Ensure `/usr/local/bin` is in your PATH, or specify the full path to the binary.

### Authentication failures with S3
- Verify `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets are set
- Check that the IAM role/user has `s3:GetObject`, `s3:PutObject`, `s3:ListBucket` permissions

### Slow uploads in CI
- Use parallel add: `mediagit add --jobs $(nproc) assets/`
- Consider using a regional S3 bucket close to your CI runners

### Repository not found
- Use `-C /path/to/repo` to specify the repository root explicitly
- Or set `MEDIAGIT_REPO` environment variable
