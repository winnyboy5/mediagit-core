#!/usr/bin/env bash
# Prevent accidentally committing large files (> 5MB)

MAX_SIZE=$((5 * 1024 * 1024))  # 5MB in bytes
large_files=0

for file in $(git diff --cached --name-only --diff-filter=ACM); do
  if [ -f "$file" ]; then
    size=$(wc -c < "$file" 2>/dev/null | tr -d ' ')
    if [ "$size" -gt "$MAX_SIZE" ] 2>/dev/null; then
      size_mb=$(echo "scale=1; $size / 1048576" | bc 2>/dev/null || echo "$((size / 1048576))")
      echo "❌ Large file: $file (${size_mb}MB > 5MB limit)"
      large_files=$((large_files + 1))
    fi
  fi
done

if [ "$large_files" -gt 0 ]; then
  echo ""
  echo "❌ $large_files file(s) exceed 5MB. Use 'git commit --no-verify' to bypass."
  exit 1
fi
