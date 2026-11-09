import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  log: vi.fn(),
  show: vi.fn(),
  branchList: vi.fn(),
  branchCreate: vi.fn(),
  branchSwitch: vi.fn(),
  branchDelete: vi.fn(),
  branchRename: vi.fn(),
  branchProtect: vi.fn(),
  branchShow: vi.fn(),
  merge: vi.fn(),
  diff: vi.fn(),
  revert: vi.fn(),
  bisectStart: vi.fn(),
  bisectMark: vi.fn(),
  bisectReset: vi.fn(),
  bisectReplay: vi.fn(),
  extract: vi.fn(),
  mediaInfo: vi.fn(),
  reflog: vi.fn(),
  reflogAll: vi.fn(),
  reflogDelete: vi.fn(),
  reflogExpire: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { ALL_REFS, useBisectStore, useReflogStore } from '@/stores/advanced'
import { useBranchesStore } from '@/stores/branches'
import { STAGED, useCompareStore } from '@/stores/compare'
import { useHistoryStore } from '@/stores/history'
import { useInspectorStore } from '@/stores/inspector'
import { useMergeStore } from '@/stores/merge'
import { useRepoStore } from '@/stores/repo'
import { useUndoStore } from '@/stores/undo'

const merged = { action: 'merge', status: 'merged', oid: 'a', conflicts: [] }

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  useRepoStore().path = '/repos/hero'
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' }, summary: {} })
  api.allowRepo.mockResolvedValue(undefined)
  api.log.mockResolvedValue({ commits: [], refs: [] })
  api.branchList.mockResolvedValue({ current: 'main', branches: [] })
  api.show.mockResolvedValue({ oid: 'o0', parents: [], files: [] })
})

describe('history filters', () => {
  it('sends only the filled fields and drops "all branches" once a start point is chosen', async () => {
    const h = useHistoryStore()
    h.filter = { revision: 'release/1', author: ' ana ', grep: '', since: '2024-01-01', until: '', paths: 'art/a.psd, video\nx.png' }
    h.applyFilter()
    await vi.waitFor(() => expect(api.log).toHaveBeenCalled())
    expect(api.log).toHaveBeenLastCalledWith('/repos/hero', 0, 200, false, undefined, {
      revision: 'release/1',
      author: 'ana',
      grep: undefined,
      since: '2024-01-01',
      until: undefined,
      paths: ['art/a.psd', 'video', 'x.png'],
    })
    expect(h.filtering).toBe(true)
    expect(h.searching).toBe(true) // the graph hides: a filtered list is not a connected history
  })

  it('clears back to the unfiltered, all-branches read', async () => {
    const h = useHistoryStore()
    h.filter.author = 'ana'
    h.applyFilter()
    await vi.waitFor(() => expect(h.filtering).toBe(true))
    h.clearFilter()
    await vi.waitFor(() => expect(h.filtering).toBe(false))
    expect(api.log).toHaveBeenLastCalledWith('/repos/hero', 0, 200, true, undefined, undefined)
  })
})

describe('branch options', () => {
  it('passes create options through and checks out a server branch with follow', async () => {
    const b = useBranchesStore()
    api.branchCreate.mockResolvedValue({})
    await b.create('fx', { startPoint: 'v1', setUpstream: 'origin/fx', tracking: 'none' })
    expect(api.branchCreate).toHaveBeenCalledWith('/repos/hero', 'fx', { startPoint: 'v1', setUpstream: 'origin/fx', tracking: 'none' })

    api.branchSwitch.mockResolvedValue({})
    await b.checkoutRemote('origin/feat')
    expect(api.branchSwitch).toHaveBeenCalledWith('/repos/hero', 'origin/feat', expect.any(Function), { create: true, track: true })
  })

  it('lists remote-tracking branches separately and keeps the local list when that fails', async () => {
    const b = useBranchesStore()
    api.branchList.mockImplementation(async (_r: string, remote?: boolean) => {
      if (remote) return { current: null, branches: [{ name: 'origin/feat', remote: true, oid: 'x' }] }
      return { current: 'main', branches: [{ name: 'main', remote: false, oid: 'y' }] }
    })
    await b.refresh()
    expect(b.list.map((x) => x.name)).toEqual(['main'])
    expect(b.remotes.map((x) => x.name)).toEqual(['origin/feat'])
    api.branchList.mockImplementation(async (_r: string, remote?: boolean) => {
      if (remote) throw new Error('nope')
      return { current: 'main', branches: [{ name: 'main', remote: false, oid: 'y' }] }
    })
    await b.refresh()
    expect(b.list).toHaveLength(1)
    expect(b.remotes).toEqual([])
  })

  it('forgets a remote branch with -r only, and protection changes go to the CLI', async () => {
    const b = useBranchesStore()
    api.branchDelete.mockResolvedValue({})
    await b.forgetRemote('origin/old')
    expect(api.branchDelete).toHaveBeenCalledWith('/repos/hero', 'origin/old', { remote: true })
    api.branchProtect.mockResolvedValue({})
    await b.protect('main', true)
    expect(api.branchProtect).toHaveBeenLastCalledWith('/repos/hero', 'main', true, false)
    await b.unprotect('main')
    expect(api.branchProtect).toHaveBeenLastCalledWith('/repos/hero', 'main', false, true)
    api.branchRename.mockResolvedValue({})
    await b.rename('a', 'b', true)
    expect(api.branchRename).toHaveBeenCalledWith('/repos/hero', 'a', 'b', true)
  })

  it('shows one branch', async () => {
    const b = useBranchesStore()
    api.branchShow.mockResolvedValue({ name: 'main', oid: 'x', protected: true })
    await b.show('main')
    expect(api.branchShow).toHaveBeenCalledWith('/repos/hero', 'main')
    expect(b.detail?.protected).toBe(true)
  })
})

describe('merge, revert and compare options', () => {
  it('merge hands every option to the CLI', async () => {
    const m = useMergeStore()
    api.merge.mockResolvedValue(merged)
    await m.start('fx', 'ours', { squash: true, noCommit: true, message: 'Bring in fx' })
    expect(api.merge).toHaveBeenCalledWith('/repos/hero', 'fx', 'ours', { squash: true, noCommit: true, message: 'Bring in fx' }, expect.any(Function))
  })

  it('reverts several versions with a message and no-commit', async () => {
    const u = useUndoStore()
    api.revert.mockResolvedValue({ action: 'revert', status: 'not_committed', commits: [], conflicts: [] })
    u.ask({ kind: 'revert', rev: 'aaaa', summary: 's', also: ['bbbb'], noCommit: true, message: ' Undo grade ' })
    await u.confirm()
    expect(api.revert).toHaveBeenCalledWith('/repos/hero', ['aaaa', 'bbbb'], { noCommit: true, message: 'Undo grade' })
  })

  it('compares the staged changes through diff --cached', async () => {
    const c = useCompareStore()
    api.log.mockResolvedValue({ commits: [{ oid: 'o1', parents: [], author: { name: 'a', email: 'a' }, time: 1, summary: 's', message: 's' }], refs: [] })
    api.diff.mockResolvedValue({ mode: 'cached', from: 'o1', to: null, files: [] })
    await c.loadCommits()
    c.to = STAGED
    await vi.waitFor(() => expect(api.diff).toHaveBeenCalledWith('/repos/hero', null, null, true))
    expect(c.staged).toBe(true)
    expect(c.working).toBe(true)
  })
})

describe('compare staged: never shows the working file as the staged one', () => {
  it('reads the before side only; the after side gets no extraction or properties', async () => {
    const c = useCompareStore()
    const inspector = useInspectorStore()
    api.log.mockResolvedValue({ commits: [{ oid: 'o1', parents: [], author: { name: 'a', email: 'a' }, time: 1, summary: 's', message: 's' }], refs: [] })
    api.diff.mockResolvedValue({ mode: 'cached', from: 'o1', to: null, files: [{ path: 'art/hero.png', status: 'modified', old_size: 10, new_size: 12, binary: true }] })
    api.extract.mockResolvedValue({ path: '/tmp/p.png', size: 10 })
    api.mediaInfo.mockResolvedValue({})
    await c.loadCommits()
    c.to = STAGED
    await vi.waitFor(() => expect(c.files).toHaveLength(1))
    c.select('art/hero.png')
    expect(api.extract).toHaveBeenCalledTimes(1)
    expect(api.extract).toHaveBeenCalledWith('/repos/hero', 'o1', 'art/hero.png', 10, expect.any(Function))
    expect(inspector.extractions.has('work:art/hero.png')).toBe(false)
    expect(inspector.infos.has('work:art/hero.png')).toBe(false)
  })
})

describe('bisect and reflog options', () => {
  it('marks another version, restarts, resets to a version and replays a log', async () => {
    const b = useBisectStore()
    const report = { status: 'testing', current: 'c', bad: 'b', good: [], skipped: [] }
    api.bisectStart.mockResolvedValue(report)
    api.bisectMark.mockResolvedValue(report)
    api.bisectReset.mockResolvedValue({ ...report, status: 'reset' })
    api.bisectReplay.mockResolvedValue(report)
    await b.start('HEAD', 'old', true)
    expect(api.bisectStart).toHaveBeenCalledWith('/repos/hero', 'HEAD', 'old', expect.any(Function), true)
    await b.mark('bad', 'abcd')
    expect(api.bisectMark).toHaveBeenCalledWith('/repos/hero', 'bad', 'abcd', expect.any(Function))
    await b.reset('abcd')
    expect(api.bisectReset).toHaveBeenCalledWith('/repos/hero', expect.any(Function), 'abcd')
    await b.replay('C:\\logs\\bisect.log')
    expect(api.bisectReplay).toHaveBeenCalledWith('/repos/hero', 'C:\\logs\\bisect.log', expect.any(Function))
  })

  it('reads every ref, and trims or forgets only through the confirmed calls', async () => {
    const r = useReflogStore()
    api.reflogAll.mockResolvedValue({ refs: [{ reference: 'main', entries: [] }] })
    r.reference = ALL_REFS
    await r.refresh()
    expect(api.reflogAll).toHaveBeenCalledWith('/repos/hero', 20)
    expect(r.groups).toHaveLength(1)

    api.reflogExpire.mockResolvedValue({ action: 'expire', changed: true, expired: 3, refs: ['main'] })
    api.reflogDelete.mockResolvedValue({ action: 'delete', changed: true, expired: 0, refs: ['main'] })
    r.reference = 'main'
    api.reflog.mockResolvedValue({ reference: 'main', entries: [] })
    await r.trim(null, 30)
    expect(api.reflogExpire).toHaveBeenCalledWith('/repos/hero', null, 30)
    await r.forget('main')
    expect(api.reflogDelete).toHaveBeenCalledWith('/repos/hero', 'main')
  })
})
