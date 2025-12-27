# mediagit add

Add file contents to the staging area for the next commit.

## Synopsis

```bash
mediagit add [OPTIONS] <PATH>...
```

## Description

Adds the current content of specified files to the staging area (index), preparing them for inclusion in the next commit. MediaGit automatically handles:

- **Content deduplication**: Identical content stored only once
- **Delta compression**: Efficient storage of file modifications
- **Chunking**: Large files split for optimal storage
- **Hash computation**: SHA-256 content addressing

## Arguments

### `<PATH>...`
Files or directories to add. Can be:
- Individual files: `video.mp4`, `image.png`
- Directories: `assets/`, `media/videos/`
- Glob patterns: `*.jpg`, `videos/*.mp4`

## Options

### `-A, --all`
Add all modified and new files in the working directory.

### `-u, --update`
Add only modified files that are already tracked (ignore new files).

### `-n, --dry-run`
Show what would be added without actually adding files.

### `-v, --verbose`
Show detailed information about added files.

### `-f, --force`
Add files even if they match `.mediagitignore` patterns.

### `--chunk-size <SIZE>`
Override default chunk size for large files.

- **Format**: `10MB`, `100MB`, `1GB`
- **Default**: `4MB`

## Examples

### Add single file
```bash
$ mediagit add video.mp4
✓ Added video.mp4 (150.2 MB → 22.5 MB after compression)
```

### Add multiple files
```bash
$ mediagit add image1.jpg image2.jpg video.mp4
✓ Added 3 files
  image1.jpg: 2.5 MB → 1.8 MB
  image2.jpg: 3.1 MB → 2.2 MB
  video.mp4: 150.2 MB → 22.5 MB
  Total: 155.8 MB → 26.5 MB (83% savings)
```

### Add entire directory
```bash
$ mediagit add assets/
✓ Added 24 files from assets/
  Compression: 840.5 MB → 156.2 MB (81.4% savings)
  Deduplication: 3 files already exist
```

### Add with glob pattern
```bash
$ mediagit add "*.psd"
✓ Added 5 PSD files
  total_design.psd: 450.2 MB → 89.1 MB
  header_mockup.psd: 120.5 MB → 28.3 MB
  ...
```

### Add all files
```bash
$ mediagit add --all
✓ Added 42 files, removed 3 deleted files
  New files: 35
  Modified files: 7
  Deleted files: 3
  Total size: 2.1 GB → 384.2 MB (81.7% savings)
```

### Dry run
```bash
$ mediagit add --dry-run *.mp4
Would add:
  video1.mp4 (150.2 MB)
  video2.mp4 (200.5 MB)
  video3.mp4 (180.3 MB)
Total: 531.0 MB (estimated: 95.6 MB after compression)
```

### Verbose output
```bash
$ mediagit add -v large_video.mp4
Processing large_video.mp4...
  Size: 1.2 GB
  Chunks: 307 (4 MB each)
  Compression: zstd level 3
  Deduplication: 12 chunks already exist
  Delta encoding: Not applicable (new file)
✓ Added large_video.mp4: 1.2 GB → 180.5 MB (84.9% savings)
  New unique chunks: 295
  Deduplicated chunks: 12
  Time: 8.3s
```

## Deduplication

MediaGit automatically deduplicates content:

```bash
$ mediagit add copy1.jpg
✓ Added copy1.jpg (5.2 MB → 3.8 MB)

$ mediagit add copy2.jpg
✓ Added copy2.jpg (5.2 MB → 0 bytes)
  ℹ File content identical to copy1.jpg (deduplicated)
```

## Large File Handling

For files >100 MB, MediaGit provides progress indicators:

```bash
$ mediagit add huge_video.mov
Adding huge_video.mov...
[████████████████████] 100% | 4.2 GB / 4.2 GB | 45s
  Chunks processed: 1075/1075
  Compression: 4.2 GB → 620.3 MB (85.2% savings)
✓ Added huge_video.mov
```

## Staging Area Status

View staged changes with `mediagit status`:

```bash
$ mediagit add video.mp4 image.jpg
$ mediagit status
On branch main

Changes to be committed:
  (use "mediagit restore --staged <file>..." to unstage)
        new file:   video.mp4
        new file:   image.jpg
```

## Exit Status

- **0**: All files added successfully
- **1**: One or more files could not be added
- **2**: Invalid options or arguments

## Notes

### Performance Tips

- **Batch operations**: Add multiple files in one command for better performance
- **Parallel processing**: MediaGit automatically uses multiple cores
- **Network optimization**: For remote backends, files are chunked and uploaded in parallel

### Storage Optimization

MediaGit optimizes storage through:

1. **Content-addressable storage**: Identical content stored once
2. **Compression**: Zstd or Brotli compression
3. **Delta encoding**: Store differences for similar files
4. **Chunking**: Efficient handling of large files

## See Also

- [mediagit status](./status.md) - Show the working tree status
- [mediagit commit](./commit.md) - Record changes to the repository
- [mediagit restore](./restore.md) - Restore working tree files
- [mediagit diff](./diff.md) - Show changes between commits
