#!/bin/sh
# MediaGit Installation Script for Unix/macOS
set -e

VERSION="${VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
REPO="yourusername/mediagit-core"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "${GREEN}MediaGit Installation Script${NC}"
echo "=============================="

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64) PLATFORM="x86_64-linux" ;;
      aarch64|arm64) PLATFORM="aarch64-linux" ;;
      *)
        echo "${RED}Error: Unsupported architecture: $ARCH${NC}"
        exit 1
        ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      x86_64) PLATFORM="x86_64-macos" ;;
      arm64) PLATFORM="aarch64-macos" ;;
      *)
        echo "${RED}Error: Unsupported architecture: $ARCH${NC}"
        exit 1
        ;;
    esac
    ;;
  *)
    echo "${RED}Error: Unsupported OS: $OS${NC}"
    echo "This script only supports Linux and macOS."
    echo "For Windows, use: iwr https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
    exit 1
    ;;
esac

# Fetch latest version if not specified
if [ "$VERSION" = "latest" ]; then
  echo "Fetching latest version..."
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/' | sed 's/^v//')
  if [ -z "$VERSION" ]; then
    echo "${RED}Error: Could not fetch latest version${NC}"
    exit 1
  fi
fi

ARCHIVE="mediagit-${VERSION}-${PLATFORM}.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE}"

echo "Platform: ${PLATFORM}"
echo "Version: ${VERSION}"
echo "Download URL: ${URL}"
echo ""

# Download and extract
echo "Downloading MediaGit ${VERSION}..."
if command -v curl > /dev/null 2>&1; then
  curl -fsSL "${URL}" | tar xz -C /tmp
elif command -v wget > /dev/null 2>&1; then
  wget -qO- "${URL}" | tar xz -C /tmp
else
  echo "${RED}Error: Neither curl nor wget is available${NC}"
  exit 1
fi

# Install binary
echo "Installing to ${INSTALL_DIR}..."
if [ -w "${INSTALL_DIR}" ]; then
  mv /tmp/mediagit "${INSTALL_DIR}/"
else
  echo "${YELLOW}Requesting sudo permissions to install to ${INSTALL_DIR}${NC}"
  sudo mv /tmp/mediagit "${INSTALL_DIR}/"
fi

chmod +x "${INSTALL_DIR}/mediagit"

# Verify installation
if command -v mediagit > /dev/null 2>&1; then
  echo ""
  echo "${GREEN}✓ MediaGit installed successfully!${NC}"
  echo ""
  echo "Installed version:"
  mediagit --version
  echo ""
  echo "Get started with:"
  echo "  mediagit init          # Initialize a new repository"
  echo "  mediagit --help        # Show all commands"
  echo ""
  echo "Documentation: https://docs.mediagit.dev"
else
  echo "${YELLOW}Warning: mediagit command not found in PATH${NC}"
  echo "You may need to add ${INSTALL_DIR} to your PATH"
fi
