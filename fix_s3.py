#!/usr/bin/env python3
"""Fix S3 backend file structure by moving helper methods out of trait impl."""

import re

# Read the file
with open('crates/mediagit-storage/src/s3.rs', 'r') as f:
    content = f.read()

# Find the corrupted section between put() and exists()
# The helper methods need to be extracted and moved after the trait impl

# Strategy:
# 1. Find line with "async fn put(" and its closing brace
# 2. Find line with "async fn exists("
# 3. Everything between them that's corrupted needs to be removed
# 4. Find where list_objects ends (should be around line 713)
# 5. Insert a new impl block for helper methods after that

lines = content.split('\n')

# Find key line numbers
put_method_start = None
exists_method_start = None
list_objects_end = None

for i, line in enumerate(lines):
    if 'async fn put(&self, key: &str, data: &[u8])' in line:
        put_method_start = i
    elif 'async fn exists(&self, key: &str)' in line:
        exists_method_start = i
    elif i > 700 and line.strip() == '}' and i < 720:  # Around line 713
        list_objects_end = i

print(f"put method starts at line {put_method_start}")
print(f"exists method starts at line {exists_method_start}")
print(f"list_objects_end around line {list_objects_end}")

# Check what's between put and exists
if put_method_start and exists_method_start:
    print(f"\nLines between put and exists ({put_method_start} to {exists_method_start}):")
    for i in range(put_method_start + 10, min(put_method_start + 20, exists_method_start)):
        print(f"{i}: {lines[i]}")
