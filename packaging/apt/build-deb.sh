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

For full documentation, visit: https://docs.mediagit.dev

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
Upstream-Contact: MediaGit Contributors <hello@mediagit.dev>
Source: https://github.com/yourusername/mediagit-core

Files: *
Copyright: 2025 MediaGit Contributors
License: AGPL-3.0
 This program is free software: you can redistribute it and/or modify
 it under the terms of the GNU Affero General Public License as published
 by the Free Software Foundation, either version 3 of the License, or
 (at your option) any later version.
 .
 This program is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY; without even the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 GNU Affero General Public License for more details.
 .
 You should have received a copy of the GNU Affero General Public License
 along with this program. If not, see <https://www.gnu.org/licenses/>.
 .
 On Debian systems, the complete text of the GNU Affero General Public
 License version 3 can be found in /usr/share/common-licenses/AGPL-3.
EOF

# Build package
dpkg-deb --build --root-owner-group "${PKG_DIR}"

echo "Package built successfully: ${PKG_DIR}.deb"
