# MediaGit Multi-Arch Docker Image
# Supports linux/amd64 and linux/arm64

FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Set up non-root user
RUN useradd -m -u 1000 -s /bin/bash mediagit

# Copy binaries based on target architecture.
# Binaries are pre-extracted by the CI workflow into docker-binaries/{amd64,arm64}/
ARG TARGETARCH
COPY --chmod=755 docker-binaries/${TARGETARCH}/mediagit /usr/local/bin/mediagit
COPY --chmod=755 docker-binaries/${TARGETARCH}/mediagit-server /usr/local/bin/mediagit-server

# Switch to non-root user
USER mediagit
WORKDIR /home/mediagit

# Set up volume for repositories
VOLUME ["/data"]

# Entrypoint
ENTRYPOINT ["/usr/local/bin/mediagit"]
CMD ["--help"]
