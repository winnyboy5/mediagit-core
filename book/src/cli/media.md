# mediagit media

Inspect media file metadata.

## Synopsis

```bash
mediagit media info <PATH> [OPTIONS]
```

## Description

Surfaces the full detail the `mediagit-media` format parsers extract from a
working-tree file — image, video, audio, PSD, and 3D-model formats. This is
richer than the one-line `media: ...` summary shown by `status`/`show`/
`stats`: every top-level field the parser produces is printed, or the whole
parsed struct as JSON.

`media` is a subcommand group; `info` is the only subcommand this cycle.

## Arguments

#### `<PATH>`
Path to the media file, resolved relative to the current directory (a
working-tree file, not an ODB object — no repository is required).

## Options

#### `--json`
Print the full parsed metadata struct as JSON instead of `label: value`
lines.

## Supported Formats

| Category | Extensions |
|----------|-----------|
| Image | `jpg`, `jpeg`, `png`, `tif`, `tiff`, `webp` |
| Video | `mp4`, `mov`, `m4v` |
| Audio | `wav`, `mp3`, `flac`, `aac`, `ogg`, `m4a` |
| PSD | `psd` |
| 3D | `obj`, `fbx`, `blend`, `gltf`, `glb`, `stl`, `usd`, `usda`, `usdc`, `usdz`, `ply` |

Files larger than 256 MB are skipped with a one-line message (the same cap
used by the `status`/`show` media summary line, gated by
`MEDIAGIT_MEDIA_META`). An unrecognized extension prints a clean one-line
message and exits `0` — this is not an error condition.

## Examples

### Image

```bash
$ mediagit media info assets/hero.png
format: Png
width: 1920
height: 1080
file_size: 2145839
perceptual_hash: 3f9a...
```

### Video, as JSON

```bash
$ mediagit media info assets/clip.mp4 --json
{
  "duration_seconds": 12.4,
  "tracks": [...],
  "video_codec": "h264",
  "audio_codec": "aac",
  "brand": "isom",
  "segments": []
}
```

### 3D model

```bash
$ mediagit media info assets/character.glb
format: Glb
vertex_count: 24831
face_count: 12400
object_count: 3
materials: 2
textures: 4
has_animations: true
has_rigging: true
```

## Exit Status

- **0**: Success (including "unsupported extension" — a clean informational
  message, not an error)
- **1**: File not found, unreadable, or malformed for its detected format

## See Also

- [mediagit status](./status.md) - Working tree status, with an optional
  `media: ...` summary line for changed media files
- [mediagit show](./show.md) - Show object information
