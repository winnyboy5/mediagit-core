import type { FileStatus } from '@/components/StatusBadge.vue'

export interface FileEntry {
  path: string
  status: FileStatus
  size?: number | null
}

export interface TreeNode {
  /** Full path; folders end without a slash. */
  path: string
  name: string
  isDir: boolean
  depth: number
  /** Own status for files; the shared status for folders, or 'modified' when mixed. */
  status: FileStatus
  /** Files below (1 for a file). */
  count: number
  /** Sum of known sizes below; null when none known. */
  size: number | null
  children: TreeNode[]
}

export interface Row {
  node: TreeNode
  expanded: boolean
}

interface Dir {
  node: TreeNode
  kids: Map<string, Dir | TreeNode>
}

function isDirEntry(x: Dir | TreeNode): x is Dir {
  return 'kids' in x
}

/** Build a tree from flat paths: folders-first, then name, case-insensitive. */
export function buildTree(entries: FileEntry[]): TreeNode[] {
  const root: Dir = { node: mk('', '', true, 0, 'modified'), kids: new Map() }
  for (const e of entries) {
    const parts = e.path.split('/').filter(Boolean)
    let cur = root
    parts.forEach((name, i) => {
      const path = parts.slice(0, i + 1).join('/')
      if (i === parts.length - 1) {
        const leaf = mk(path, name, false, i, e.status)
        leaf.count = 1
        leaf.size = e.size ?? null
        cur.kids.set(name, leaf)
        return
      }
      let next = cur.kids.get(name)
      if (!next || !isDirEntry(next)) {
        next = { node: mk(path, name, true, i, e.status), kids: new Map() }
        cur.kids.set(name, next)
      }
      cur = next
    })
  }
  return finish(root).children
}

function mk(path: string, name: string, isDir: boolean, depth: number, status: FileStatus): TreeNode {
  return { path, name, isDir, depth, status, count: 0, size: null, children: [] }
}

// Post-order: sort, then aggregate count / size / status.
function finish(d: Dir): TreeNode {
  const n = d.node
  const kids = [...d.kids.values()].map((k) => (isDirEntry(k) ? finish(k) : k))
  kids.sort((a, b) => Number(b.isDir) - Number(a.isDir) || cmp(a.name, b.name))
  n.children = kids
  n.count = 0
  n.size = null
  let status: FileStatus | null = null
  let mixed = false
  for (const k of kids) {
    n.count += k.count
    if (k.size != null) n.size = (n.size ?? 0) + k.size
    if (status === null) status = k.status
    else if (status !== k.status) mixed = true
  }
  if (kids.length) n.status = mixed ? 'modified' : (status as FileStatus)
  return n
}

function cmp(a: string, b: string): number {
  const x = a.toLowerCase()
  const y = b.toLowerCase()
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0
}

/** Visible rows for a tree given the collapsed folder set (default expanded). */
export function flattenTree(roots: TreeNode[], collapsed: ReadonlySet<string>): Row[] {
  const out: Row[] = []
  const walk = (nodes: TreeNode[]) => {
    for (const n of nodes) {
      const expanded = n.isDir && !collapsed.has(n.path)
      out.push({ node: n, expanded })
      if (expanded) walk(n.children)
    }
  }
  walk(roots)
  return out
}

/** Flat mode: one row per file, same row shape, no nesting. */
export function flatRows(entries: FileEntry[]): Row[] {
  // Decorate-sort-undecorate: lower-casing once per entry, not once per compare.
  return entries
    .map((e) => ({ e, k: e.path.toLowerCase() }))
    .sort((a, b) => (a.k < b.k ? -1 : a.k > b.k ? 1 : a.e.path < b.e.path ? -1 : a.e.path > b.e.path ? 1 : 0))
    .map(({ e }) => ({
      node: { path: e.path, name: e.path, isDir: false, depth: 0, status: e.status, count: 1, size: e.size ?? null, children: [] },
      expanded: false,
    }))
}

/** Every file path under a node (a file yields itself). */
export function filesUnder(n: TreeNode): string[] {
  if (!n.isDir) return [n.path]
  return n.children.flatMap(filesUnder)
}
