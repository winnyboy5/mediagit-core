#!/bin/bash
# Start all test services for MediaGit integration tests

set -e

echo "🚀 Starting MediaGit test services..."

# Check if docker-compose is available
if ! command -v docker-compose &> /dev/null; then
    echo "❌ docker-compose not found. Please install Docker and docker-compose."
    exit 1
fi

# Start services in detached mode
docker-compose up -d

echo "⏳ Waiting for services to be healthy..."

# Wait for all services to be healthy
max_attempts=30
attempt=0

while [ $attempt -lt $max_attempts ]; do
    if docker-compose ps | grep -q "unhealthy"; then
        echo "  Services still starting... (attempt $((attempt + 1))/$max_attempts)"
        sleep 2
        attempt=$((attempt + 1))
    else
        echo "✅ All services are healthy!"

        echo ""
        echo "📊 Service Status:"
        docker-compose ps

        echo ""
        echo "🔗 Service Endpoints:"
        echo "  MinIO:               http://localhost:9000 (console: http://localhost:9001)"
        echo "  Azurite (Blob):      http://localhost:10000  [profile: azure]"
        echo "  Fake GCS:            http://localhost:4443   [profile: gcs]"

        echo ""
        echo "🧪 Ready to run integration tests:"
        echo "  MinIO (local S3):    dev-tests/deep-tests/deep_test_minio.ps1"
        echo "  AWS / Azure / GCS:   dev-tests/deep-tests/deep_test_aws|azure|gcs.ps1"

        exit 0
    fi
done

echo "❌ Services failed to become healthy after $max_attempts attempts"
echo "Check logs with: docker-compose logs"
exit 1
