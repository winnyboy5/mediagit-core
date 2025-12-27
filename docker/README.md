# Docker Configurations

This directory contains Docker and Docker Compose configurations for MediaGit-Core services.

## Available Configurations

### MinIO (S3-Compatible Storage)

**File**: `docker-compose-minio.yml`

**Purpose**: Local S3-compatible storage for development and testing

**Usage**:
```bash
# Start MinIO
docker-compose -f docker/docker-compose-minio.yml up -d

# Check status
docker ps | grep minio

# Access MinIO Console
# URL: http://localhost:9001
# Username: minioadmin
# Password: minioadmin

# Stop MinIO
docker-compose -f docker/docker-compose-minio.yml down
```

**Ports**:
- `9000`: S3 API endpoint
- `9001`: Web console

**Validation**: MinIO tested at 108.69 MB/s upload, 263.15 MB/s download

## Main Docker Compose Files

The root directory contains additional docker-compose files:

- `docker-compose.yml`: Main application services
- `docker-compose.test.yml`: Testing environment

## Development Workflow

1. Start MinIO for S3 testing:
   ```bash
   docker-compose -f docker/docker-compose-minio.yml up -d
   ```

2. Configure MediaGit to use MinIO (see DEVELOPMENT_GUIDE.md)

3. Run tests:
   ```bash
   ./tests/minio_cloud_backend_test.sh
   ```

4. Stop services when done:
   ```bash
   docker-compose -f docker/docker-compose-minio.yml down
   ```

## Notes

- MinIO data is persisted in Docker volume `minio_data`
- Default credentials are for development only - change for production
- See `../DEVELOPMENT_GUIDE.md` for complete MinIO setup guide
