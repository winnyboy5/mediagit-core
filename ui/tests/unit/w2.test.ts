import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { DiffReport, MergeReport } from '@/api'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  branchList: vi.fn(),
  merge: vi.fn(),
  mergeControl: vi.fn(),
  mergeResolve: vi.fn(),
  diff: vi.fn(),
  log: vi.fn(),
  extract: vi.fn(),
  mediaInfo: vi.fn(),
  stashList: vi.fn(),
  stashSave: vi.fn(),
  stashApply: vi.fn(),
  stashPop: vi.fn(),
  stashDrop: vi.fn(),
  stashClear: vi.fn(),
  tagList: vi.fn(),
  tagCreate: vi.fn(),
  tagDelete: vi.fn(),
  reset: vi.fn(),
  resetHard: vi.fn(),
  revert: vi.fn(),
  revertControl: vi.fn(),
  restoreFile: vi.fn(),
  show: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { metaRows } from '@/lib/compare'
import { conflictReason, mergeSummary, missingSide, toConflicts } from '@/lib/merge'
import { useBranchesStore } from '@/stores/branches'
import { useCompareStore, WORKING } from '@/stores/compare'
import { useMergeStore } from '@/stores/merge'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useTagsStashStore } from '@/stores/tagsStash'
import { useUndoStore } from '@/stores/undo'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' }, summary: { staged: 0, modified: 0, deleted: 0, untracked: 0 } })
  api.allowRepo.mockResolvedValue(undefined)
  api.branchList.mockResolvedValue({ current: 'main', branches: [{ name: 'main', oid: 'aa', remote: false }, { name: 'fx', oid: 'bb', remote: false }] })
  api.log.mockResolvedValue({ commits: [], refs: [] })
  api.show.mockResolvedValue({ oid: 'o0', parents: [], files: [] })
})

const open = () => {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  return repo
}

const conflict: MergeReport = {
  action: 'merge',
  status: 'conflict',
  oid: null,
  conflicts: [
    { path: 'a.psd', state: 'unresolved', kind: 'modify_modify' },
    { path: 'b.fbx', state: 'unresolved', kind: 'modify_delete' },
  ],
}

describe('merge lib', () => {
  it('words each conflict kind and names the side that has no file', () => {
    expect(conflictReason('modify_modify')).toBe('Both changed')
    expect(conflictReason('add_add')).toBe('Both added')
    expect(conflictReason('weird')).toBe('Needs a decision')
    expect(missingSide('delete_modify')).toBe('ours')
    expect(missingSide('modify_delete')).toBe('theirs')
    expect(missingSide('modify_modify')).toBeNull()
  })

  it('treats a path the CLI already shows as staged as resolved', () => {
    expect(toConflicts([{ path: 'a', state: 'unresolved', kind: 'add_add' }, { path: 'b', state: 'staged', kind: 'add_add' }]).map((c) => c.resolved)).toEqual([null, 'staged'])
  })

  it('summarises every merge outcome in words', () => {
    expect(mergeSummary(conflict)).toBe('Stopped: 2 files need a decision')
    expect(mergeSummary({ ...conflict, conflicts: [conflict.conflicts[0]!] })).toBe('Stopped: 1 file needs a decision')
    expect(mergeSummary({ action: 'merge', status: 'fast_forward', oid: 'x', conflicts: [] })).toBe('Fast-forwarded')
    expect(mergeSummary({ action: 'abort', status: 'aborted', oid: null, conflicts: [] })).toBe('Merge cancelled')
    expect(mergeSummary({ action: 'merge', status: 'new_status', oid: null, conflicts: [] })).toBe('new_status')
  })
})

describe('compare lib', () => {
  it('lists the size first, then properties from either side, flagging what changed', () => {
    const rows = metaRows({ width: 100, format: 'png' }, { width: 200, format: 'png', layers: 3 }, { before: 1024, after: 2048 })
    expect(rows.map((r) => [r.label, r.before, r.after, r.changed])).toEqual([
      ['Size', '1.0 KB', '2.0 KB', true],
      ['Width', '100', '200', true],
      ['Format', 'png', 'png', false],
      ['Layers', '–', '3', true],
    ])
  })

  it('copes with a missing side or missing sizes', () => {
    expect(metaRows(undefined, undefined, { before: null, after: null })).toEqual([{ label: 'Size', before: '–', after: '–', changed: false }])
    expect(metaRows(undefined, undefined, { before: null, after: 5 })[0]).toMatchObject({ before: '–', after: '5 B', changed: true })
  })
})

describe('merge store', () => {
  it('starts a merge, keeps the conflicts, and remembers their commit for "theirs"', async () => {
    open()
    const branches = useBranchesStore()
    await branches.refresh()
    const m = useMergeStore()
    api.merge.mockResolvedValue(conflict)
    expect(await m.start('fx', 'recursive', {})).toBe(true)
    expect(api.merge).toHaveBeenCalledWith('/repos/hero', 'fx', 'recursive', {}, expect.any(Function))
    expect(m.active).toBe(true)
    expect(m.theirsName).toBe('fx')
    expect(m.theirsOid).toBe('bb')
    expect(m.conflicts.map((c) => c.path)).toEqual(['a.psd', 'b.fbx'])
    expect(m.canContinue).toBe(false)
  })

  it('resolves a file with the chosen side and can continue only when all are decided', async () => {
    open()
    await useBranchesStore().refresh()
    const m = useMergeStore()
    api.merge.mockResolvedValue(conflict)
    await m.start('fx', 'recursive', {})
    api.mergeResolve.mockResolvedValue(null)
    expect(await m.resolve('a.psd', 'theirs')).toBe(true)
    expect(api.mergeResolve).toHaveBeenCalledWith('/repos/hero', 'a.psd', 'theirs', 'bb')
    expect(m.resolved).toBe(1)
    expect(m.canContinue).toBe(false)
    await m.resolve('b.fbx', 'ours')
    expect(m.canContinue).toBe(true)

    api.mergeControl.mockResolvedValue({ action: 'continue', status: 'merged', oid: 'cc', conflicts: [] })
    expect(await m.continue()).toBe(true)
    expect(api.mergeControl).toHaveBeenCalledWith('/repos/hero', 'continue', expect.any(Function))
    expect(m.active).toBe(false)
    expect(m.conflicts).toEqual([])
    expect(m.last?.status).toBe('merged')
  })

  it('refuses "theirs" when the merge was not started here, and keeps the file undecided on failure', async () => {
    open()
    const m = useMergeStore()
    expect(await m.resolve('a.psd', 'theirs')).toBe(false)
    expect(api.mergeResolve).not.toHaveBeenCalled()

    await useBranchesStore().refresh()
    api.merge.mockResolvedValue(conflict)
    await m.start('fx', 'ours', {})
    api.mergeResolve.mockRejectedValue({ code: 'error', message: 'disk full' })
    expect(await m.resolve('a.psd', 'ours')).toBe(false)
    expect(m.error).toEqual({ code: 'error', message: 'disk full' })
    expect(m.conflicts[0]!.resolved).toBeNull()
  })

  it('abort clears the merge; a merge with no conflicts never becomes active', async () => {
    open()
    await useBranchesStore().refresh()
    const m = useMergeStore()
    api.merge.mockResolvedValue({ action: 'merge', status: 'fast_forward', oid: 'x', conflicts: [] })
    await m.start('fx', 'recursive', {})
    expect(m.active).toBe(false)
    api.merge.mockResolvedValue(conflict)
    await m.start('fx', 'recursive', {})
    expect(m.active).toBe(true)
    api.mergeControl.mockResolvedValue({ action: 'abort', status: 'aborted', oid: null, conflicts: [] })
    await m.abort()
    expect(api.mergeControl).toHaveBeenCalledWith('/repos/hero', 'abort', expect.any(Function))
    expect(m.active).toBe(false)
    expect(useOpsStore().finished.map((o) => o.label)).toContain('Cancel merge')
  })

  it('does not start a second write while one runs', async () => {
    open()
    await useBranchesStore().refresh()
    const m = useMergeStore()
    let release!: (r: MergeReport) => void
    api.merge.mockImplementation(() => new Promise((r) => (release = r)))
    const first = m.start('fx', 'recursive', {})
    await nextTick()
    expect(m.busy).toBe(true)
    expect(await m.start('fx', 'recursive', {})).toBe(false)
    release(conflict)
    await first
    expect(api.merge).toHaveBeenCalledTimes(1)
  })

  it('forgets everything when the repository changes', async () => {
    const repo = open()
    await useBranchesStore().refresh()
    const m = useMergeStore()
    api.merge.mockResolvedValue(conflict)
    await m.start('fx', 'recursive', {})
    repo.path = '/repos/other'
    await nextTick()
    expect(m.active).toBe(false)
    expect(m.conflicts).toEqual([])
    expect(m.last).toBeNull()
  })
})

describe('tags and stash store', () => {
  it('loads tags and stashes independently; one failing leaves the other', async () => {
    open()
    const s = useTagsStashStore()
    api.tagList.mockRejectedValue({ code: 'error', message: 'no tags' })
    api.stashList.mockResolvedValue({ stashes: [{ index: 0, oid: 'a', message: 'm', branch: 'main', timestamp: 't' }] })
    await s.refresh()
    expect(s.tagsError?.message).toBe('no tags')
    expect(s.tagsLoaded).toBe(false)
    expect(s.stashLoaded).toBe(true)
    expect(s.stashes).toHaveLength(1)
  })

  it('saves with a trimmed message (null when blank) and refreshes the lists afterwards', async () => {
    open()
    const s = useTagsStashStore()
    api.tagList.mockResolvedValue({ tags: [] })
    api.stashList.mockResolvedValue({ stashes: [] })
    api.stashSave.mockResolvedValue({ oid: 'a' })
    await s.save('  WIP  ', true)
    expect(api.stashSave).toHaveBeenLastCalledWith('/repos/hero', 'WIP', true, [], expect.any(Function))
    await s.save('   ', false)
    expect(api.stashSave).toHaveBeenLastCalledWith('/repos/hero', null, false, [], expect.any(Function))
    expect(api.stashList).toHaveBeenCalledTimes(2)
  })

  it('apply, pop, drop and clear each call their own command with the entry index', async () => {
    open()
    const s = useTagsStashStore()
    api.tagList.mockResolvedValue({ tags: [] })
    api.stashList.mockResolvedValue({ stashes: [] })
    api.stashApply.mockResolvedValue({ files_updated: 3 })
    api.stashPop.mockResolvedValue({ files_updated: 1 })
    api.stashDrop.mockResolvedValue({})
    api.stashClear.mockResolvedValue({})
    await s.apply(2)
    await s.pop(1)
    await s.drop(0)
    await s.clear()
    expect(api.stashApply).toHaveBeenCalledWith('/repos/hero', 2, {}, expect.any(Function))
    expect(api.stashPop).toHaveBeenCalledWith('/repos/hero', 1, {}, expect.any(Function))
    expect(api.stashDrop).toHaveBeenCalledWith('/repos/hero', 0)
    expect(api.stashClear).toHaveBeenCalledWith('/repos/hero')
    expect(useOpsStore().finished.map((o) => o.summary)).toContain('Applied to 3 files')
  })

  it('creates a tag on the latest version (null commit, trimmed note) and deletes by name', async () => {
    open()
    const s = useTagsStashStore()
    api.tagList.mockResolvedValue({ tags: [] })
    api.stashList.mockResolvedValue({ stashes: [] })
    api.tagCreate.mockResolvedValue({ action: 'create', created: null, deleted: [], missing: [] })
    api.tagDelete.mockResolvedValue({ action: 'delete', created: null, deleted: [], missing: ['v1'] })
    await s.createTag('v1', { message: ' note ' })
    await s.createTag('v2', { message: '  ' })
    expect(api.tagCreate.mock.calls).toEqual([
      ['/repos/hero', 'v1', { message: 'note' }],
      ['/repos/hero', 'v2', { message: null }],
    ])
    await s.deleteTag('v1')
    expect(api.tagDelete).toHaveBeenCalledWith('/repos/hero', 'v1')
    expect(useOpsStore().finished.map((o) => o.summary)).toContain('Tag was already gone')
    expect(api.tagList).toHaveBeenCalledTimes(3)
  })

  it('keeps a failed action in actionError', async () => {
    open()
    const s = useTagsStashStore()
    api.tagList.mockResolvedValue({ tags: [] })
    api.stashList.mockResolvedValue({ stashes: [] })
    api.stashPop.mockRejectedValue({ code: 'error', message: 'clash' })
    expect(await s.pop(0)).toBeNull()
    expect(s.actionError?.message).toBe('clash')
  })
})

describe('undo store', () => {
  it('does nothing until a dialog is confirmed', async () => {
    open()
    const u = useUndoStore()
    expect(await u.confirm()).toBe(false)
    u.ask({ kind: 'reset', mode: 'soft', rev: 'abcd' })
    expect(u.open).toBe(true)
    expect(api.reset).not.toHaveBeenCalled()
  })

  it('soft and mixed resets use `reset`, hard uses `resetHard`; history reloads after', async () => {
    open()
    const u = useUndoStore()
    api.reset.mockResolvedValue({ mode: 'soft', oid: 'abcd' })
    api.resetHard.mockResolvedValue({ mode: 'hard', oid: 'abcd' })
    u.ask({ kind: 'reset', mode: 'soft', rev: 'abcd' })
    await u.confirm()
    u.ask({ kind: 'reset', mode: 'mixed', rev: 'abcd' })
    await u.confirm()
    expect(api.reset.mock.calls.map((c) => c.slice(0, 3))).toEqual([
      ['/repos/hero', 'abcd', 'soft'],
      ['/repos/hero', 'abcd', 'mixed'],
    ])
    expect(api.resetHard).not.toHaveBeenCalled()
    u.ask({ kind: 'reset', mode: 'hard', rev: 'HEAD' })
    await u.confirm()
    expect(api.resetHard).toHaveBeenCalledWith('/repos/hero', 'HEAD', true, expect.any(Function))
    expect(api.reset).toHaveBeenCalledTimes(2)
    expect(api.log).toHaveBeenCalled()
    expect(u.open).toBe(false)
  })

  it('a revert that clashes keeps its files for the banner; abort clears them', async () => {
    open()
    const u = useUndoStore()
    api.revert.mockResolvedValue({ action: 'revert', status: 'conflict', commits: [], conflicts: ['a.psd'] })
    u.ask({ kind: 'revert', rev: 'abcd', summary: 's' })
    await u.confirm()
    expect(u.revertConflicts).toEqual(['a.psd'])
    api.revertControl.mockResolvedValue({ action: 'abort', status: 'aborted', commits: [], conflicts: [] })
    await u.revertControl('abort')
    expect(api.revertControl).toHaveBeenCalledWith('/repos/hero', 'abort')
    expect(u.revertConflicts).toEqual([])
  })

  it('restores one file at a version and reports a failure', async () => {
    open()
    const u = useUndoStore()
    api.restoreFile.mockRejectedValue({ code: 'error', message: 'locked' })
    u.ask({ kind: 'restore', rev: 'abcd', path: 'art/a.png' })
    expect(await u.confirm()).toBe(false)
    expect(api.restoreFile).toHaveBeenCalledWith('/repos/hero', 'abcd', 'art/a.png')
    expect(u.error?.message).toBe('locked')
  })
})

describe('compare store', () => {
  const C = (oid: string) => ({ oid, parents: [], author: { name: 'M', email: 'm@x' }, time: 1, summary: oid, message: oid })
  const report = (over: Partial<DiffReport> = {}): DiffReport => ({
    mode: 'working',
    from: 'h1',
    to: null,
    files: [
      { path: 'a.png', status: 'modified', old_size: 1, new_size: 2, binary: true },
      { path: 'new.png', status: 'added', old_size: null, new_size: 3, binary: true },
      { path: 'gone.png', status: 'deleted', old_size: 4, new_size: null, binary: true },
    ],
    ...over,
  })

  async function ready() {
    open()
    api.log.mockResolvedValue({ commits: [C('h1'), C('h0')], refs: [] })
    api.diff.mockResolvedValue(report())
    api.extract.mockResolvedValue({ path: '/p/x.png', size: 1 })
    api.mediaInfo.mockResolvedValue({})
    const c = useCompareStore()
    await c.loadCommits()
    await nextTick()
    await vi.waitFor(() => expect(c.report).not.toBeNull())
    return c
  }

  it('starts on working folder against the latest version', async () => {
    const c = await ready()
    expect(c.from).toBe('h1')
    expect(c.to).toBe(WORKING)
    expect(api.diff).toHaveBeenCalledWith('/repos/hero', null, null, false)
    expect(c.files).toHaveLength(3)
  })

  it('compares two versions with exactly those ids, and skips the same version', async () => {
    const c = await ready()
    c.to = 'h0'
    await vi.waitFor(() => expect(api.diff).toHaveBeenLastCalledWith('/repos/hero', 'h1', 'h0'))
    const calls = api.diff.mock.calls.length
    c.from = 'h0'
    await nextTick()
    expect(api.diff.mock.calls.length).toBe(calls)
    expect(c.files).toEqual([])
  })

  it('selecting a file prepares both sides: HEAD extracted, working side read in place', async () => {
    const c = await ready()
    c.select('a.png')
    expect(c.sides.before).toEqual({ path: 'a.png', size: 1, rev: 'h1' })
    expect(c.sides.after).toEqual({ path: 'a.png', size: 2, rev: null })
    await vi.waitFor(() => expect(api.extract).toHaveBeenCalledTimes(1))
    expect(api.extract).toHaveBeenCalledWith('/repos/hero', 'h1', 'a.png', 1, expect.any(Function))
  })

  it('added files have no before side and deleted files no after side', async () => {
    const c = await ready()
    c.select('new.png')
    expect(c.sides.before).toBeNull()
    expect(c.sides.after).not.toBeNull()
    c.select('gone.png')
    expect(c.sides.before).not.toBeNull()
    expect(c.sides.after).toBeNull()
  })

  it('does not extract files that are neither images nor have a metadata parser', async () => {
    const c = await ready()
    api.diff.mockResolvedValue(report({ files: [{ path: 'notes.txt', status: 'modified', old_size: 1, new_size: 2, binary: true }] }))
    await c.compare()
    c.select('notes.txt')
    await nextTick()
    expect(api.extract).not.toHaveBeenCalled()
  })

  it('keeps an error from the comparison', async () => {
    const c = await ready()
    api.diff.mockRejectedValue({ code: 'error', message: 'bad version' })
    await c.compare()
    expect(c.error?.message).toBe('bad version')
  })
})
