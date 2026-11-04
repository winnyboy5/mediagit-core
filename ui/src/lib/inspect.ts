import { formatBytes } from '@/components/format'
import { extOf } from '@/components/media'
import type { FileStatus } from '@/components/StatusBadge.vue'

/** Extraction above this shows determinate progress in the inspector. */
export const EXTRACT_PROGRESS_MIN = 20 * 1024 * 1024

export type PreviewKind = 'image' | 'video' | 'audio' | 'none'

// Only formats a system WebView plays natively; PSD, EXR, TIFF, 3D etc. get icon + metadata.
const IMAGE = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg', 'avif'])
const VIDEO = new Set(['mp4', 'm4v', 'webm', 'ogv', 'mov'])
const AUDIO = new Set(['wav', 'mp3', 'flac', 'ogg', 'm4a', 'aac', 'opus'])

export function previewKind(path: string): PreviewKind {
  const e = extOf(path)
  return IMAGE.has(e) ? 'image' : VIDEO.has(e) ? 'video' : AUDIO.has(e) ? 'audio' : 'none'
}

/** `color_mode` -> `Color mode`. */
export function humanizeKey(key: string): string {
  const s = key.replace(/[_\-.]+/g, ' ').trim()
  return s ? s[0]!.toUpperCase() + s.slice(1) : key
}

export interface InfoRow {
  label: string
  value: string
}

function scalar(key: string, v: unknown): string | null {
  if (v == null || v === '') return null
  if (typeof v === 'boolean') return v ? 'Yes' : 'No'
  if (typeof v === 'number') {
    if (/(^|_)(size|bytes)$/.test(key) && Number.isInteger(v)) return formatBytes(v)
    return Number.isInteger(v) ? String(v) : String(Math.round(v * 100) / 100)
  }
  return typeof v === 'string' ? v : null
}

/**
 * Flatten a parser's JSON into label/value rows. The shape differs per file type,
 * so this is generic: scalars as-is, short arrays joined (a pair reads as a size),
 * objects one level deep with the parent's name as prefix.
 */
export function flattenInfo(info: Record<string, unknown>, limit = 24): InfoRow[] {
  const rows: InfoRow[] = []
  const add = (key: string, prefix: string, v: unknown, depth: number) => {
    if (rows.length >= limit) return
    const label = humanizeKey(prefix ? `${prefix} ${key}` : key)
    const s = scalar(key, v)
    if (s != null) return void rows.push({ label, value: s })
    if (Array.isArray(v)) {
      const items = v.map((x) => scalar(key, x))
      if (items.length && items.length <= 8 && items.every((x) => x != null)) {
        rows.push({ label, value: items.join(items.length === 2 && v.every((x) => typeof x === 'number') ? ' × ' : ', ') })
      }
    } else if (v && typeof v === 'object' && depth < 2) {
      for (const [k, x] of Object.entries(v)) add(k, prefix ? `${prefix} ${key}` : key, x, depth + 1)
    }
  }
  for (const [k, v] of Object.entries(info)) add(k, '', v, 0)
  return rows
}

/** Why a branch or remote name is not allowed, or null. Mirrors the desktop crate's check. */
export function refNameError(name: string): string | null {
  if (!name.trim()) return 'Enter a name'
  if (name.length > 255) return 'Use 255 characters or fewer'
  if (name.startsWith('-')) return 'Cannot start with a dash'
  if (/\s/.test(name)) return 'Cannot contain spaces'
  // eslint-disable-next-line no-control-regex
  if (/[\x00-\x1f\x7f~^:?*[\\]/.test(name)) return 'Contains a character that is not allowed'
  if (name.startsWith('/') || name.endsWith('/') || name.includes('//')) return 'Slashes must separate names'
  if (name.includes('..') || name.includes('@{') || name.endsWith('.') || name.endsWith('.lock')) return 'That name is reserved'
  return null
}

/** What the inspector describes: a working-copy file (`rev` null) or a file at a version. */
export interface InspectFile {
  path: string
  size: number | null
  rev: string | null
  status?: FileStatus
  group?: 'staged' | 'unstaged'
  /** In a version: the file was deleted by it, so there is nothing to open. */
  deleted?: boolean
}

/** Join a repo root and a `/`-separated relative path using the root's separator. */
export function joinPath(root: string, rel: string): string {
  const win = root.includes('\\')
  return root.replace(/[\\/]+$/, '') + (win ? '\\' : '/') + (win ? rel.replace(/\//g, '\\') : rel)
}

// Types `media info` has a parser for (the CLI refuses the rest).
const INFO_EXT = new Set([
  'jpg', 'jpeg', 'png', 'tif', 'tiff', 'webp', 'mp4', 'mov', 'm4v', 'wav', 'mp3', 'flac', 'aac', 'ogg', 'm4a', 'psd',
  'obj', 'fbx', 'blend', 'gltf', 'glb', 'stl', 'usd', 'usda', 'usdc', 'usdz', 'ply', 'ai', 'ait', 'indd', 'indt', 'aep', 'aet', 'prproj',
])
export const hasMediaInfo = (path: string) => INFO_EXT.has(extOf(path))
