#!/usr/bin/env bash
# AGPL-3.0 License Header Check
# Ensures all staged .rs files have the required AGPL license header.

missing=0
for file in $(git diff --cached --name-only --diff-filter=ACM -- '*.rs'); do
  if ! grep -q "GNU Affero General Public License" "$file" 2>/dev/null; then
    echo "❌ Missing license header: $file"
    missing=$((missing + 1))
  fi
done

if [ "$missing" -gt 0 ]; then
  echo "❌ $missing file(s) missing AGPL-3.0 license headers"
  exit 1
fi
echo "✅ All staged .rs files have license headers"
