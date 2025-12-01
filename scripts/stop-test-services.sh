#!/bin/bash
# Stop all test services and clean up

set -e

echo "🛑 Stopping MediaGit test services..."

docker-compose down

echo "✅ All services stopped"

# Optionally clean up volumes
if [ "$1" == "--clean" ]; then
    echo "🧹 Cleaning up volumes..."
    docker-compose down -v
    echo "✅ Volumes cleaned"
fi
