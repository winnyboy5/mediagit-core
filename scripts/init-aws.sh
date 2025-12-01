#!/bin/bash
# Initialize LocalStack with test S3 buckets for MediaGit integration tests

set -e

echo "Waiting for LocalStack to be ready..."
sleep 5

echo "Creating test S3 buckets..."

# Create test buckets
aws --endpoint-url=http://localhost:4566 \
    --region=us-east-1 \
    s3 mb s3://mediagit-test-bucket || true

aws --endpoint-url=http://localhost:4566 \
    --region=us-east-1 \
    s3 mb s3://mediagit-integration-tests || true

echo "S3 buckets created successfully!"

# List buckets to verify
aws --endpoint-url=http://localhost:4566 \
    --region=us-east-1 \
    s3 ls

echo "LocalStack initialization complete!"
