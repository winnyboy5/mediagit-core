import { describe, expect, it } from 'vitest'
import { buildTree, filesUnder, flatRows, flattenTree, type FileEntry } from '@/lib/fileTree'

const E = (path: string, status: FileEntry['status'] = 'modified', size?: number): FileEntry => ({ path, status, size })

describe('buildTree', () => {
  it('nests by path and puts folders before files, case-insensitively', () => {
    const t = buildTree([E('b.txt'), E('Z/a.psd'), E('a/x/y.exr'), E('A.txt')])
    expect(t.map((n) => n.name)).toEqual(['a', 'Z', 'A.txt', 'b.txt'])
    expect(t[0]!.children[0]!.name).toBe('x')
    expect(t[0]!.children[0]!.children[0]!.depth).toBe(2)
  })

  it('is stable regardless of input order', () => {
    const a = [E('d/2'), E('d/1'), E('c'), E('e/f/g')]
    const names = (xs: FileEntry[]) => flattenTree(buildTree(xs), new Set()).map((r) => r.node.path)
    expect(names(a)).toEqual(names([...a].reverse()))
  })

  it('aggregates count, size and status per folder', () => {
    const t = buildTree([E('d/a', 'added', 10), E('d/b', 'added', 5), E('d/s/c', 'added'), E('m/x', 'added'), E('m/y', 'deleted')])
    const d = t.find((n) => n.name === 'd')!
    expect(d.count).toBe(3)
    expect(d.size).toBe(15)
    expect(d.status).toBe('added')
    const m = t.find((n) => n.name === 'm')!
    expect(m.status).toBe('modified') // mixed
    expect(m.size).toBeNull() // no sizes known: never invent one
  })

  it('handles empty input and a file named like a folder prefix', () => {
    expect(buildTree([])).toEqual([])
    const t = buildTree([E('a'), E('a.b/c')])
    expect(t.map((n) => [n.name, n.isDir])).toEqual([
      ['a.b', true],
      ['a', false],
    ])
  })
})

describe('flattenTree / flatRows / filesUnder', () => {
  const tree = buildTree([E('d/a'), E('d/s/c'), E('top')])

  it('hides children of collapsed folders', () => {
    expect(flattenTree(tree, new Set()).map((r) => r.node.path)).toEqual(['d', 'd/s', 'd/s/c', 'd/a', 'top'])
    expect(flattenTree(tree, new Set(['d'])).map((r) => r.node.path)).toEqual(['d', 'top'])
    expect(flattenTree(tree, new Set(['d/s'])).map((r) => r.node.path)).toEqual(['d', 'd/s', 'd/a', 'top'])
  })

  it('flat rows are files only, sorted by path', () => {
    expect(flatRows([E('z/1'), E('a/2')]).map((r) => r.node.path)).toEqual(['a/2', 'z/1'])
  })

  it('lists files under a folder', () => {
    expect(filesUnder(tree[0]!).sort()).toEqual(['d/a', 'd/s/c'])
    expect(filesUnder(tree[1]!)).toEqual(['top'])
  })
})

describe('100k paths', () => {
  const entries: FileEntry[] = Array.from({ length: 100_000 }, (_, i) =>
    E(`shots/sh${String(i % 500).padStart(3, '0')}/seq${i % 40}/frame_${String(i).padStart(6, '0')}.exr`, 'added', 1000),
  )

  it('builds and flattens without blowing up', () => {
    const t0 = performance.now()
    const tree = buildTree(entries)
    const rows = flattenTree(tree, new Set())
    expect(tree[0]!.count).toBe(100_000)
    expect(rows.length).toBeGreaterThan(100_000)
    expect(performance.now() - t0).toBeLessThan(3000)
  })
})
