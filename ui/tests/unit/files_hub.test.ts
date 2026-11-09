import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  add: vi.fn(),
  unstage: vi.fn(),
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  lsFiles: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { buildTree, flattenTree, type FileEntry } from '@/lib/fileTree'
import { includeAction, mergeChanges, ProgressJoin, stagingStats, tickOf } from '@/lib/staging'
import { useChangesStore } from '@/stores/changes'
import { filterEntries, toEntries, useFilesStore } from '@/stores/files'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({
    branch: { name: 'main', upstream: null },
    staged: [{ path: 'art/a.psd', kind: 'added' }],
    modified: ['art/b.psd', 'art/a.psd'],
    deleted: [],
    untracked: ['c.png'],
  })
  api.allowRepo.mockResolvedValue(undefined)
})

async function open() {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  await repo.refresh()
}

describe('include mapping', () => {
  it('a file staged then edited again is one included change', () => {
    const list = mergeChanges([{ path: 'a', status: 'added' }], [{ path: 'a', status: 'modified' }, { path: 'b', status: 'untracked' }])
    expect(list.map((e) => [e.path, e.included])).toEqual([['a', true], ['b', false]])
  })

  it('folders are on, off or mixed by the files below; clicking a mixed or off row includes it', () => {
    const tree = buildTree(mergeChanges([{ path: 'art/a.psd', status: 'added' }], [{ path: 'art/b.psd', status: 'modified' }, { path: 'c.png', status: 'untracked' }]))
    const art = tree.find((n) => n.path === 'art')!
    expect(tickOf(art)).toBe('indeterminate')
    expect(tickOf(tree.find((n) => n.path === 'c.png')!)).toBe(false)
    expect(tickOf(art.children[0]!)).toBe(true)
    expect(includeAction(tickOf(art))).toBe('stage')
    expect(includeAction(true)).toBe('unstage')
    expect(includeAction(false)).toBe('stage')
  })

  it('checking stages the path, unchecking unstages it, Include all stages a dot', async () => {
    await open()
    const c = useChangesStore()
    expect(c.includedCount).toBe(1)
    expect(c.list).toHaveLength(3)
    api.add.mockResolvedValue({ added: 1, modified: 0, deleted: 0, bytes: 0 })
    api.unstage.mockResolvedValue({})
    await c.toggleInclude('c.png')
    expect(api.add).toHaveBeenLastCalledWith('/repos/hero', ['c.png'], expect.anything(), expect.any(Function))
    await c.toggleInclude('art')
    expect(api.add).toHaveBeenLastCalledWith('/repos/hero', ['art'], expect.anything(), expect.any(Function))
    await c.stageAll()
    expect(api.add).toHaveBeenLastCalledWith('/repos/hero', ['.'], expect.anything(), expect.any(Function))
    await c.toggleInclude('art/a.psd')
    expect(api.unstage).toHaveBeenLastCalledWith('/repos/hero', ['art/a.psd'])
  })

  it('selected rows are included or excluded together', async () => {
    await open()
    const c = useChangesStore()
    api.add.mockResolvedValue({ added: 0, modified: 0, deleted: 0, bytes: 0 })
    api.unstage.mockResolvedValue({})
    c.select('c.png')
    c.select('art/b.psd', { ctrl: true })
    expect(c.selectedCount).toBe(2)
    await c.includeSelected()
    expect(api.add.mock.calls[0]![1]).toEqual(['c.png', 'art/b.psd'])
    c.select('c.png')
    await c.excludeSelected()
    expect(api.unstage).toHaveBeenLastCalledWith('/repos/hero', ['c.png'])
  })
})

describe('staging progress', () => {
  it('folds finished calls into whole-job totals and keys bars by unit', () => {
    const j = new ProgressJoin()
    const a = j.map({ id: 7, msg: 'x', pos: 40, len: 100, unit: 'bytes', done: false })
    expect(a).toMatchObject({ id: 0, pos: 40, len: 100 })
    j.map({ id: 9, msg: '', pos: 2, len: 2, unit: 'items', done: false })
    j.next()
    const b = j.map({ id: 7, msg: 'y', pos: 10, len: 50, unit: 'bytes', done: false })
    expect(b).toMatchObject({ id: 0, pos: 50, len: 150 })
    expect(j.map({ id: 9, msg: '', pos: 1, len: 3, unit: 'items', done: false })).toMatchObject({ id: 1, pos: 3, len: 5 })
  })

  it('reports files, bytes, rate and ETA only once measured', () => {
    const ops = useOpsStore()
    const id = ops.begin({ kind: 'add', label: 'Staging 10 files' }, 0)
    ops.progress(id, { id: 0, msg: 'a', pos: 0, len: 1000, unit: 'bytes', done: false }, 0)
    expect(stagingStats(ops.running[0]!)).toMatchObject({ files: null, rate: null, eta: null, pct: 0 })
    ops.progress(id, { id: 1, msg: 'files', pos: 3, len: 10, unit: 'items', done: false }, 500)
    ops.progress(id, { id: 0, msg: 'a', pos: 500, len: 1000, unit: 'bytes', done: false }, 2000)
    const s = stagingStats(ops.running[0]!)
    expect(s.files).toEqual({ done: 3, total: 10 })
    expect(s.bytes).toEqual({ done: 500, total: 1000 })
    expect(s.rate).toBeCloseTo(250)
    expect(s.eta).toBeCloseTo(2)
    expect(s.pct).toBe(50)
  })
})

describe('explorer', () => {
  const files = [
    { path: 'art/a.psd', status: 'modified', staged: true, size: 10 },
    { path: 'art/b.psd', status: 'unchanged', staged: false, size: 20 },
    { path: 'z/c.png', status: 'new', staged: false, size: 5 },
    { path: 'z/.cache', status: 'ignored', staged: false, size: 1 },
    { path: 'gone.txt', status: 'deleted', staged: false, size: null },
  ] as const

  it('maps ls-files: only changed files get a status; ignored are dimmed', () => {
    const e = toEntries([...files])
    expect(e.map((x) => x.status)).toEqual(['modified', null, 'untracked', null, 'deleted'])
    expect(e[3]!.ignored).toBe(true)
    expect(e[0]!.included).toBe(true)
  })

  it('folder rows count changed files', () => {
    const rows = flattenTree(buildTree(toEntries([...files])), new Set())
    const art = rows.find((r) => r.node.path === 'art')!.node
    expect([art.count, art.changed]).toEqual([2, 1])
  })

  it('find matches every word of a path, case-insensitively', () => {
    const e = toEntries([...files])
    expect(filterEntries(e, 'ART psd').map((x) => x.path)).toEqual(['art/a.psd', 'art/b.psd'])
    expect(filterEntries(e, '  ')).toBe(e)
    expect(filterEntries(e, 'nope')).toEqual([])
  })

  it('loads through ls-files with the ignored switch and reloads on toggle', async () => {
    api.lsFiles.mockResolvedValue({ files: [...files] })
    await open()
    const f = useFilesStore()
    await f.refresh()
    expect(api.lsFiles).toHaveBeenLastCalledWith('/repos/hero', false)
    expect(f.changedCount).toBe(3)
    f.showIgnored = true
    await vi.waitFor(() => expect(api.lsFiles).toHaveBeenLastCalledWith('/repos/hero', true))
    f.select('art/a.psd')
    expect(f.selectedFiles).toEqual([{ path: 'art/a.psd', status: 'modified', group: 'staged', size: 10 }])
  })

  it('builds the tree for 100,000 entries in under a second', () => {
    const e: FileEntry[] = Array.from({ length: 100_000 }, (_, i) => ({ path: `shots/s${i % 500}/f${i}.exr`, status: i % 50 === 0 ? 'modified' : null, size: i }))
    const t0 = performance.now()
    const rows = flattenTree(buildTree(filterEntries(e, '')), new Set())
    const ms = performance.now() - t0
    expect(rows.length).toBeGreaterThan(100_000)
    expect(ms).toBeLessThan(1000)
    const t1 = performance.now()
    filterEntries(e, 's42 f4')
    expect(performance.now() - t1).toBeLessThan(250)
  })
})
