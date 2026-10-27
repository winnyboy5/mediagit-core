import { AudioLines, Box, File, FileText, Film, Image, Layers, Sparkles, type LucideIcon } from 'lucide-vue-next'

export type MediaKind = 'image' | 'video' | 'audio' | 'model3d' | 'document' | 'vfx' | 'other'

export interface MediaInfo {
  kind: MediaKind
  icon: LucideIcon
  /** CSS custom property holding the kind's hue, e.g. `--media-image`. */
  hueVar: `--media-${MediaKind}`
  /** Matching soft tint, e.g. `--media-image-soft`. */
  softVar: `--media-${MediaKind}-soft`
}

const EXT: Record<MediaKind, string[]> = {
  image: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'tif', 'tiff', 'tga', 'svg', 'heic', 'avif', 'dng', 'raw', 'cr2', 'nef', 'arw'],
  video: ['mp4', 'mov', 'mxf', 'mkv', 'avi', 'webm', 'm4v', 'mpg', 'mpeg', 'r3d', 'braw', 'prores', 'dnxhd', 'wmv', 'ogv'],
  audio: ['wav', 'aif', 'aiff', 'mp3', 'flac', 'ogg', 'm4a', 'aac', 'opus', 'bwf'],
  model3d: ['fbx', 'obj', 'blend', 'usd', 'usda', 'usdc', 'usdz', 'abc', 'gltf', 'glb', 'stl', 'ply', 'dae', '3ds', 'max', 'ma', 'mb', 'c4d', 'hip', 'hipnc', 'ztl', 'zpr', 'uasset', 'umap'],
  vfx: ['exr', 'vdb', 'nk', 'nknc', 'cube', 'ocio', 'dpx', 'hdr', 'cin', 'aep', 'drp', 'comp'],
  document: ['psd', 'psb', 'pdf', 'ai', 'kra', 'xcf', 'clip', 'indd', 'afphoto', 'sketch', 'fig', 'doc', 'docx', 'txt', 'md'],
  other: [],
}

const LAYERED = new Set(['psd', 'psb', 'kra', 'xcf', 'clip', 'ai', 'afphoto'])

const ICON: Record<MediaKind, LucideIcon> = {
  image: Image,
  video: Film,
  audio: AudioLines,
  model3d: Box,
  document: FileText,
  vfx: Sparkles,
  other: File,
}

const BY_EXT = new Map<string, MediaKind>()
for (const [kind, exts] of Object.entries(EXT) as [MediaKind, string[]][]) for (const e of exts) BY_EXT.set(e, kind)

export function extOf(path: string): string {
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1)
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
}

export function mediaKind(path: string): MediaInfo {
  const ext = extOf(path)
  const kind = BY_EXT.get(ext) ?? 'other'
  return {
    kind,
    icon: LAYERED.has(ext) ? Layers : ICON[kind],
    hueVar: `--media-${kind}`,
    softVar: `--media-${kind}-soft`,
  }
}
