#!/usr/bin/env bash
# bump-version.sh — Update the project version everywhere.
#
# Usage:
#   ./scripts/bump-version.sh 0.2.1
#   ./scripts/bump-version.sh 0.3.0
#
# What it does:
#   1. Updates workspace version in Cargo.toml (all crates inherit it)
#   2. Updates all hardcoded version strings in docs (.md files)
#   3. Updates example config files
#   4. Prints a summary of changes
#
# After running, review with `git diff`, then commit and tag:
#   git add -A && git commit -m "chore: bump version to v$NEW"
#   git tag -a "v$NEW" -m "Release v$NEW"
#   git push origin main "v$NEW"

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "Usage: $0 <new-version>"
    echo "Example: $0 0.3.0"
    exit 1
fi

NEW="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Read current version from Cargo.toml
OLD=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')

if [ "$OLD" = "$NEW" ]; then
    echo "Version is already $NEW — nothing to do."
    exit 0
fi

echo "Bumping version: $OLD → $NEW"
echo "Root: $ROOT"
echo ""

# 1. Cargo.toml workspace version (the single Rust source of truth)
sed -i "0,/^version = \"$OLD\"/s//version = \"$NEW\"/" "$ROOT/Cargo.toml"
echo "  ✓ Cargo.toml workspace version"

# 2. Documentation files — replace version in URLs, archive names, examples
#    Pattern: replace OLD version in download URLs, archive names, docker tags, etc.
#    We use word-boundary-safe replacements to avoid corrupting CHANGELOG history.
DOC_FILES=$(find "$ROOT" \
    -name '*.md' \
    -not -path '*/target/*' \
    -not -path '*/.git/*' \
    -not -path '*/CHANGELOG.md' \
    -not -path '*/claudedocs/*')

count=0
for f in $DOC_FILES; do
    if grep -q "$OLD" "$f" 2>/dev/null; then
        sed -i "s/$OLD/$NEW/g" "$f"
        count=$((count + 1))
        echo "  ✓ $(realpath --relative-to="$ROOT" "$f")"
    fi
done
echo "  Updated $count doc files"

# 3. Example config files
EXAMPLE_CONFIGS=$(find "$ROOT/crates" -name '*.toml' -path '*/examples/*' 2>/dev/null || true)
for f in $EXAMPLE_CONFIGS; do
    if grep -q "$OLD" "$f" 2>/dev/null; then
        sed -i "s/$OLD/$NEW/g" "$f"
        echo "  ✓ $(realpath --relative-to="$ROOT" "$f")"
    fi
done

# 4. CHANGELOG.md — only update the [Unreleased] compare link
if [ -f "$ROOT/CHANGELOG.md" ]; then
    sed -i "s|compare/v$OLD\.\.\.HEAD|compare/v$NEW...HEAD|" "$ROOT/CHANGELOG.md"
    echo "  ✓ CHANGELOG.md [Unreleased] link"
fi

echo ""
echo "Done! Version bumped from $OLD to $NEW."
echo ""
echo "Next steps:"
echo "  1. Review changes:  git diff"
echo "  2. Update CHANGELOG.md with new release notes"
echo "  3. Commit:          git add -A && git commit -m 'chore: bump version to v$NEW'"
echo "  4. Tag:             git tag -a v$NEW -m 'Release v$NEW'"
echo "  5. Push:            git push origin main v$NEW"
