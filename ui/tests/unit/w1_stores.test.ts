import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { LogCommitReport } from '@/api'

const api = vi.hoisted(() => ({
  log: vi.fn(),
  show: vi.fn(),
  extract: vi.fn(),
  mediaInfo: vi.fn(),
  lockList: vi.fn(),
  lockCreate: vi.fn(),
  lockUnlock: vi.fn(),
  branchList: vi.fn(),
  branchSwitch: vi.fn(),
  branchCreate: vi.fn(),
  remoteList: vi.fn(),
  remoteAdd: vi.fn(),
  push: vi.fn(),
  fetch: vi.fn(),
  pull: vi.fn(),
  cancelOp: vi.fn(),
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { PAGE_SIZE, useHistoryStore } from '@/stores/history'
import { fileKey, useInspectorStore } from '@/stores/inspector'
import { useLocksStore } from '@/stores/locks'
import { useBranchesStore } from '@/stores/branches'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

const C = (i: number, parents: number[] = [i + 1]): LogCommitReport => ({
  oid: `o${i}`,
  parents: parents.map((p) => `o${p}`),
  author: { name: 'Mira', email: 'm@x.example' },
  time: 1000 - i,
  summary: `commit ${i}`,
  message: `commit ${i}`,
})
const page = (from: number, n: number) => Array.from({ length: n }, (_, k) => C(from + k))

function openRepo() {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  return repo
}

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main', upstream: { name: 'origin/main', ahead: 1, behind: 0 } } })
  api.allowRepo.mockResolvedValue(undefined)
  api.show.mockResolvedValue({ oid: 'o0', parents: [], files: [] })
})

describe('history store', () => {
  it('pages 200 at a time, keeps lanes stable across pages, and stops at a short page', async () => {
    openRepo()
    const h = useHistoryStore()
    api.log.mockResolvedValueOnce({ commits: page(0, PAGE_SIZE), refs: [{ name: 'HEAD', oid: 'o0', kind: 'head' }] })
    await h.reload()
    expect(api.log).toHaveBeenLastCalledWith('/repos/hero', 0, PAGE_SIZE, true)
    expect(h.visible).toHaveLength(PAGE_SIZE)
    const firstRows = h.rows.slice(0, 5)
    expect(h.done).toBe(false)

    api.log.mockResolvedValueOnce({ commits: page(PAGE_SIZE, 30), refs: [] })
    await h.loadMore()
    expect(api.log).toHaveBeenLastCalledWith('/repos/hero', PAGE_SIZE, PAGE_SIZE, true)
    expect(h.visible).toHaveLength(PAGE_SIZE + 30)
    expect(h.rows.slice(0, 5)).toEqual(firstRows)
    expect(h.done).toBe(true)

    await h.loadMore()
    expect(api.log).toHaveBeenCalledTimes(2) // done: no third request
  })

  it('selects the newest commit after the first load and fetches its detail once', async () => {
    openRepo()
    const h = useHistoryStore()
    api.log.mockResolvedValue({ commits: page(0, 3), refs: [] })
    await h.reload()
    await nextTick()
    expect(h.selectedOid).toBe('o0')
    expect(api.show).toHaveBeenCalledWith('/repos/hero', 'o0')
    await h.select('o0')
    expect(api.show).toHaveBeenCalledTimes(1)
  })

  it('"this branch only" keeps HEAD ancestors and re-lays the graph out; search hides the graph', async () => {
    openRepo()
    const h = useHistoryStore()
    // o0 (HEAD) -> o2; o1 is a side branch tip -> o2
    api.log.mockResolvedValue({
      commits: [C(0, [2]), C(1, [2]), C(2, [])],
      refs: [{ name: 'HEAD', oid: 'o0', kind: 'head' }],
    })
    await h.reload()
    expect(h.visible.map((c) => c.oid)).toEqual(['o0', 'o1', 'o2'])
    h.branchOnly = true
    expect(h.visible.map((c) => c.oid)).toEqual(['o0', 'o2'])
    expect(h.rows).toHaveLength(2)
    h.branchOnly = false
    h.query = 'commit 1'
    expect(h.visible.map((c) => c.oid)).toEqual(['o1'])
    expect(h.rows).toEqual([])
  })

  it('keeps old rows on a failed reload and reports the error', async () => {
    openRepo()
    const h = useHistoryStore()
    api.log.mockResolvedValueOnce({ commits: page(0, 2), refs: [] })
    await h.reload()
    api.log.mockRejectedValueOnce({ code: 'error', message: 'disk gone' })
    await h.reload()
    expect(h.error?.message).toBe('disk gone')
    expect(h.visible).toHaveLength(2)
  })

  it('loads the tree only on demand and once', async () => {
    openRepo()
    const h = useHistoryStore()
    api.log.mockResolvedValue({ commits: page(0, 2), refs: [] })
    await h.reload()
    api.show.mockResolvedValue({ oid: 'o0', parents: [], files: [], tree: [{ path: 'a.png', size: 5, oid: 'x' }] })
    await h.loadTree()
    await h.loadTree()
    expect(api.show.mock.calls.filter((c) => c[2] === true)).toHaveLength(1)
    expect(h.tree).toEqual([{ path: 'a.png', size: 5, oid: 'x' }])
    h.selectFile({ path: 'a.png', size: 5, deleted: false })
    expect(h.inspectFile).toMatchObject({ path: 'a.png', rev: 'o0' })
  })
})

describe('inspector store', () => {
  it('uses the working file directly and extracts a version file once, with progress', async () => {
    openRepo()
    const i = useInspectorStore()
    i.ensureExtraction({ path: 'art/a.png', size: 10, rev: null })
    expect(i.extractions.get(fileKey({ path: 'art/a.png', rev: null }))).toMatchObject({ status: 'ready', path: '/repos/hero/art/a.png' })

    let report: (p: { pos: number; len: number | null }) => void = () => {}
    let finish: (v: { path: string; size: number }) => void = () => {}
    api.extract.mockImplementation((_r, _rev, _p, _size, cb) => {
      report = cb
      return new Promise((res) => (finish = res))
    })
    const f = { path: 'v/big.mp4', size: 100, rev: 'abc1' }
    i.ensureExtraction(f)
    i.ensureExtraction(f)
    expect(api.extract).toHaveBeenCalledTimes(1)
    report({ pos: 40, len: 100 })
    expect(i.extractions.get(fileKey(f))).toMatchObject({ status: 'loading', pos: 40, len: 100 })
    finish({ path: '/tmp/x.mp4', size: 100 })
    await vi.waitFor(() => expect(i.extractions.get(fileKey(f))?.status).toBe('ready'))
    expect(i.extractions.get(fileKey(f))?.path).toBe('/tmp/x.mp4')
  })

  it('reads media info from the extracted blob for a version, and records failures', async () => {
    openRepo()
    const i = useInspectorStore()
    api.extract.mockResolvedValue({ path: '/tmp/x.png', size: 1 })
    api.mediaInfo.mockResolvedValue({ width: 4 })
    const f = { path: 'a.png', size: 1, rev: 'abc1' }
    i.ensureInfo(f)
    await vi.waitFor(() => expect(i.infos.get(fileKey(f))?.status).toBe('ready'))
    expect(api.mediaInfo).toHaveBeenCalledWith('/repos/hero', '/tmp/x.png')

    api.mediaInfo.mockRejectedValue({ code: 'error', message: 'Unsupported media type' })
    const w = { path: 'b.png', size: 1, rev: null }
    i.ensureInfo(w)
    await vi.waitFor(() => expect(i.infos.get(fileKey(w))?.status).toBe('error'))
    expect(api.mediaInfo).toHaveBeenLastCalledWith('/repos/hero', 'b.png')
  })

  it('asks for a file history once per path', async () => {
    openRepo()
    const i = useInspectorStore()
    api.log.mockResolvedValue({ commits: page(0, 2), refs: [] })
    i.ensureHistory({ path: 'a.png', size: 1, rev: null })
    i.ensureHistory({ path: 'a.png', size: 1, rev: 'x' })
    expect(api.log).toHaveBeenCalledTimes(1)
    expect(api.log).toHaveBeenCalledWith('/repos/hero', 0, 10, false, 'a.png')
  })
})

describe('locks store', () => {
  it('lists sorted by path, remembers my owner name from the first lock, and refreshes after each action', async () => {
    openRepo()
    const l = useLocksStore()
    l.me = null
    api.lockList.mockResolvedValue({ locks: [{ lock_id: '2', path: 'b', owner: 'dev', created_at: 1 }, { lock_id: '1', path: 'a', owner: 'dev', created_at: 1 }] })
    await l.refresh()
    expect(l.locks.map((x) => x.path)).toEqual(['a', 'b'])
    expect(l.byPath.get('b')?.lock_id).toBe('2')

    api.lockCreate.mockResolvedValue({ lock_id: '3', path: 'c', owner: 'mira', created_at: 2 })
    expect(await l.lock('c')).toBe(true)
    expect(api.lockCreate).toHaveBeenCalledWith('/repos/hero', 'c')
    expect(l.me).toBe('mira')
    expect(api.lockList).toHaveBeenCalledTimes(2)
  })

  it('surfaces a refusal without throwing and does not learn an owner', async () => {
    openRepo()
    const l = useLocksStore()
    l.me = null
    api.lockList.mockResolvedValue({ locks: [] })
    api.lockCreate.mockRejectedValue({ code: 'error', message: 'Already locked by dev' })
    expect(await l.lock('a')).toBe(false)
    expect(l.actionError?.message).toBe('Already locked by dev')
    expect(l.me).toBeNull()
    expect(l.busy).toBeNull()
  })

  it('force unlock passes the flag', async () => {
    openRepo()
    const l = useLocksStore()
    api.lockList.mockResolvedValue({ locks: [] })
    api.lockUnlock.mockResolvedValue({})
    await l.unlock({ lock_id: '9', path: 'p', owner: 'dev', created_at: 0 }, true)
    expect(api.lockUnlock).toHaveBeenCalledWith('/repos/hero', '9', true)
  })
})

describe('branches store', () => {
  it('lists local branches only and switches through a tracked op that streams items', async () => {
    openRepo()
    const b = useBranchesStore()
    api.branchList.mockResolvedValue({
      current: 'main',
      branches: [
        { name: 'main', remote: false, protected: true },
        { name: 'origin/main', remote: true, protected: false },
      ],
    })
    await b.refresh()
    expect(b.list.map((x) => x.name)).toEqual(['main'])

    let seen: string | undefined
    api.branchSwitch.mockImplementation(async (_r, _n, report) => {
      report({ id: 1, msg: 'Updating files', pos: 5, len: 40, unit: 'items', done: false })
      seen = b.switching?.phase
      return {}
    })
    expect(await b.switchTo('feature/grade')).toBe(true)
    expect(seen).toBe('Updating files')
    expect(b.switching).toBeUndefined()
    expect(useOpsStore().finished[0]?.summary).toBe('Switched to feature/grade')
  })

  it('keeps a dirty-tree refusal as plain text and stays usable', async () => {
    openRepo()
    const b = useBranchesStore()
    api.branchList.mockResolvedValue({ current: 'main', branches: [] })
    api.branchSwitch.mockRejectedValue({ code: 'error', message: 'You have unsaved changes in 3 files.' })
    expect(await b.switchTo('x')).toBe(false)
    expect(b.actionError).toEqual({ code: 'error', message: 'You have unsaved changes in 3 files.' })
    expect(b.busy).toBe(false)
  })
})

describe('sync store', () => {
  const remotes = { remotes: [{ name: 'origin', url: 'https://h.example/r' }, { name: 'backup', url: '/mnt/b' }] }

  it('picks the upstream remote, then origin, unless the user chose', async () => {
    openRepo()
    const repo = useRepoStore()
    await repo.refresh()
    const s = useSyncStore()
    api.remoteList.mockResolvedValue(remotes)
    await s.refresh()
    expect(s.remote).toBe('origin')
    s.chosen = 'backup'
    expect(s.remote).toBe('backup')
    s.chosen = 'gone'
    expect(s.remote).toBe('origin')
  })

  it('push is a cancellable tracked op: progress in, cancel hits the backend, success stamps last sync', async () => {
    const repo = openRepo()
    await repo.refresh()
    const s = useSyncStore()
    api.remoteList.mockResolvedValue(remotes)
    await s.refresh()
    let opId = ''
    api.push.mockImplementation(async (_r, _remote, id, report) => {
      opId = id
      report({ id: 2, msg: 'Uploading media', pos: 10, len: 100, unit: 'bytes', done: false })
      expect(s.active?.kind).toBe('push')
      await s.active?.cancel?.()
      return { result: 'pushed', objects_sent: 2, bytes_uploaded: 10 }
    })
    api.cancelOp.mockResolvedValue(true)
    expect(await s.run('push')).toBe(true)
    expect(api.push).toHaveBeenCalledWith('/repos/hero', 'origin', expect.stringMatching(/^op-/), expect.any(Function))
    expect(api.cancelOp).toHaveBeenCalledWith(opId)
    expect(s.last).toBeGreaterThan(0)
    expect(s.active).toBeUndefined()
  })

  it('a cancelled transfer is not an error; a failed one is, and leaves a Retry in the drawer', async () => {
    const repo = openRepo()
    await repo.refresh()
    const s = useSyncStore()
    api.remoteList.mockResolvedValue(remotes)
    await s.refresh()
    api.pull.mockRejectedValueOnce({ code: 'cancelled', message: 'Cancelled' })
    expect(await s.run('pull')).toBe(false)
    expect(s.actionError).toBeNull()
    api.fetch.mockRejectedValueOnce({ code: 'E_NET', message: 'timed out' })
    expect(await s.run('fetch')).toBe(false)
    expect(s.actionError?.code).toBe('E_NET')
    const failed = useOpsStore().finished.find((o) => o.status === 'failed')
    expect(failed?.retry).toBeTypeOf('function')
  })

  it('refuses a second mutating action while one runs', async () => {
    const repo = openRepo()
    await repo.refresh()
    const s = useSyncStore()
    api.remoteList.mockResolvedValue(remotes)
    await s.refresh()
    let release: () => void = () => {}
    api.fetch.mockImplementation(() => new Promise((r) => (release = () => r({ branches_updated: 0, tags_updated: 0, bytes_downloaded: 0 }))))
    const first = s.run('fetch')
    expect(s.busy).toBe(true)
    expect(await s.run('push')).toBe(false)
    expect(api.push).not.toHaveBeenCalled()
    release()
    await first
  })
})
