#!/bin/bash
# Build Debian package for MediaGit

set -e

VERSION="${VERSION:-0.1.0}"
ARCH="${ARCH:-amd64}"  # amd64 or arm64
MAINTAINER="MediaGit Contributors <hello@mediagit.dev>"

# Create package structure
PKG_DIR="mediagit_${VERSION}_${ARCH}"
mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/share/doc/mediagit"
mkdir -p "${PKG_DIR}/usr/share/man/man1"

# Create control file
cat > "${PKG_DIR}/DEBIAN/control" << EOF
Package: mediagit
Version: ${VERSION}
Section: vcs
Priority: optional
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Description: Git-based version control for large media files
 MediaGit is a modern version control system optimized for managing
 large binary files such as images, videos, 3D models, and machine
 learning datasets.
 .
 Features include:
  * Fast branch switching (under 100ms)
  * Intelligent compression and deduplication
  * Media-aware merge capabilities
  * Support for multiple cloud storage backends
  * Git-compatible workflow
Homepage: https://mediagit.dev
EOF

# Copy binary (should be downloaded from release)
if [ -f "mediagit" ]; then
    cp mediagit "${PKG_DIR}/usr/bin/"
    chmod 755 "${PKG_DIR}/usr/bin/mediagit"
else
    echo "Error: mediagit binary not found"
    exit 1
fi

# Copy documentation
cat > "${PKG_DIR}/usr/share/doc/mediagit/README" << 'EOF'
MediaGit - Git-based Version Control for Large Media Files

For full documentation, visit: https://winnyboy5.github.io/mediagit-core

Quick Start:
  mediagit init           Initialize a new repository
  mediagit add <file>     Add files to staging
  mediagit commit         Create a commit
  mediagit branch <name>  Create a new branch
  mediagit merge <branch> Merge branches

For more information:
  mediagit --help
EOF

# Create changelog
cat > "${PKG_DIR}/usr/share/doc/mediagit/changelog.Debian" << EOF
mediagit (${VERSION}) unstable; urgency=medium

  * Release version ${VERSION}

 -- ${MAINTAINER}  $(date -R)
EOF

gzip -9 "${PKG_DIR}/usr/share/doc/mediagit/changelog.Debian"

# Copy copyright
cat > "${PKG_DIR}/usr/share/doc/mediagit/copyright" << 'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: mediagit
Upstream-Contact: Aswin Krishnamoorthy <licensing@mediagit.dev>
Source: https://github.com/winnyboy5/mediagit-core

Files: *
Copyright: 2025-2026 Aswin Krishnamoorthy
License: BUSL-1.1
 Business Source License 1.1. This is a source-available licence, NOT an
 OSI-approved open source licence, and this package is therefore non-free by
 Debian's definition.
 .
 Production use is permitted at any scale and by any organisation, provided
 that use does not include offering the Licensed Work, or a service whose
 primary value derives from it, to third parties as a hosted, managed, or
 software-as-a-service offering.
 .
 Four years after a given version is published, that version becomes available
 under the Change Licence, the GNU Affero General Public License version 3 or
 later.
 .
 The full licence text is installed alongside this file as LICENSE, and is
 available at https://github.com/winnyboy5/mediagit-core/blob/main/LICENSE
 .
 For commercial licensing, contact licensing@mediagit.dev
EOF

# Build package
dpkg-deb --build --root-owner-group "${PKG_DIR}"

echo "Package built successfully: ${PKG_DIR}.deb"
