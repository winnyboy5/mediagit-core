# Media-Aware Merging

**Status: conflict detection only — not an auto-merge.** MediaGit can parse
structured media formats to tell whether two sets of edits overlap, but it
cannot write a merged file back in any of them. PSD writing is unsupported by
the parser MediaGit uses, and video/audio would need re-encoding — an
auto-merge that can't produce a real output file isn't an auto-merge.

## Supported Formats (inspection only)
- **PSD/PSB**: `mediagit media info` reports layer names, dimensions, colour mode
- **Video** (MP4, MOV, ...): reports streams, codecs, duration
- **Audio**: reports track/channel structure

## How It Works
1. Parse file format structure on each side of the merge
2. Identify layers/tracks/channels that changed
3. If both sides touched the same file, report a conflict — it is never silently resolved
4. Check out one side into the working tree so the file stays valid (conflict markers can't be inlined into binary content)

## Example: PSD Conflict
```bash
# Branch A: Added "Background" layer
# Branch B: Added "Foreground" layer
mediagit merge feature-branch

# Result: conflict recorded, one side checked out.
# You resolve by producing the file you want (e.g. in Photoshop),
# then `mediagit add` it to clear the conflict.
```

See [User Guide - Merging Media](../guides/merging-media.md) for the resolution workflow.
