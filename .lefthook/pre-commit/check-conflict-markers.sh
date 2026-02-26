#!/usr/bin/env bash
# Check for leftover merge conflict markers in staged files

conflicts=0
for file in $(git diff --cached --name-only --diff-filter=ACM); do
  if grep -qE "^(<<<<<<<|=======|>>>>>>>)" "$file" 2>/dev/null; then
    echo "❌ Conflict markers found in: $file"
    conflicts=$((conflicts + 1))
  fi
done

if [ "$conflicts" -gt 0 ]; then
  echo ""
  echo "❌ $conflicts file(s) contain merge conflict markers. Resolve them before committing."
  exit 1
fi
