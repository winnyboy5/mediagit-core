import type { RemoteReport } from '@/api'

export interface Crumb {
  label: string
  /** Folder path this crumb opens; empty for the root. */
  path: string
}

/** `video/shots` -> root, `video`, `video/shots`. */
export function breadcrumbs(rootLabel: string, path: string): Crumb[] {
  const parts = path.split('/').filter(Boolean)
  return [{ label: rootLabel, path: '' }, ...parts.map((label, i) => ({ label, path: parts.slice(0, i + 1).join('/') }))]
}

/**
 * What `download` is told to fetch. It resolves bare paths against `origin`, so any other
 * http(s) remote is addressed by its full URL.
 */
export function downloadTarget(remote: Pick<RemoteReport, 'name' | 'url'>, path: string): string {
  if (remote.name === 'origin' || !/^https?:\/\//.test(remote.url)) return path
  return `${remote.url.replace(/\/+$/, '')}/${path}`
}

/** Join a chosen folder and a file name with the folder's own separator. */
export function joinOut(folder: string, name: string): string {
  const sep = folder.includes('\\') ? '\\' : '/'
  return folder.replace(/[\\/]+$/, '') + sep + name
}

/** Folders first, then files, each by name. */
export function sortEntries<T extends { name: string; kind: string }>(list: readonly T[]): T[] {
  return [...list].sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'dir' ? -1 : 1))
}
