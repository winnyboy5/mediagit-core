# Media-Aware Merging

MediaGit understands structured media formats for intelligent merging.

## Supported Formats
- **PSD/PSB**: Layer-aware merging
- **Video**: Track-based merging (planned)
- **Audio**: Channel-aware merging (planned)

## How It Works
1. Parse file format structure
2. Identify layers/tracks/channels
3. Merge at structural level
4. Preserve hierarchy and metadata

## Example: PSD Merging
```bash
# Branch A: Added "Background" layer
# Branch B: Added "Foreground" layer
mediagit merge feature-branch

# Result: Both layers preserved in merged file
```

See [User Guide - Merging Media](../guides/merging-media.md) for examples.
