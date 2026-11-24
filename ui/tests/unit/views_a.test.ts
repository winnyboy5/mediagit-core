import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  add: vi.fn(),
  cancelOp: vi.fn(),
  commit: vi.fn(),
  unstage: vi.fn(),
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  remoteList: vi.fn(),
  remoteAdd: vi.fn(),
  remoteRename: vi.fn(),
  remoteSetUrl: vi.fn(),
  remoteShow: vi.fn(),
  push: vi.fn(),
  pull: vi.fn(),
  fetch: vi.fn(),
  lockList: vi.fn(),
  lockCreate: vi.fn(),
  lockUnlock: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { emptyForm, parseRefspecs, pushIsRisky, transferOpts } from '@/lib/sync'
import { useChangesStore } from '@/stores/changes'
import { useLocksStore } from '@/stores/locks'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main', upstream: null }, staged: [{ path: 'a.psd', kind: 'added' }], modified: [], deleted: [], untracked: ['b.psd'] })
  api.allowRepo.mockResolvedValue(undefined)
  api.remoteList.mockResolvedValue({ remotes: [{ name: 'origin', url: 'https://h.example/r' }] })
  api.lockList.mockResolvedValue({ locks: [] })
})

async function open() {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  await repo.refresh()
}

describe('transfer options', () => {
  it('a fresh form sends nothing extra and is not risky', () => {
    const f = emptyForm()
    expect(transferOpts('fetch', f)).toEqual({})
    expect(transferOpts('pull', f)).toEqual({})
    expect(transferOpts('push', f)).toEqual({})
    expect(pushIsRisky(f.push)).toBe(false)
  })

  it('deleting ignores an overwrite choice left over from before', () => {
    const f = emptyForm()
    Object.assign(f.push, { refspecs: 'old', delete: true, overwrite: 'force' })
    expect(transferOpts('push', f, true)).toEqual({ refspecs: ['old'], delete: true, confirmed: true })
  })

  it('maps every ticked option', () => {
    const f = emptyForm()
    Object.assign(f.fetch, { branch: ' dev ', prune: true })
    expect(transferOpts('fetch', f)).toEqual({ branch: 'dev', prune: true })
    f.fetch.all = true
    expect(transferOpts('fetch', f)).toEqual({ all: true, prune: true })
    Object.assign(f.pull, { branch: 'dev', rebase: true, dryRun: true })
    expect(transferOpts('pull', f)).toEqual({ branch: 'dev', rebase: true, dryRun: true })
    Object.assign(f.push, { refspecs: 'main, dev:rel', tags: true, followTags: true, dryRun: true, setUpstream: true, repair: true })
    expect(transferOpts('push', f)).toEqual({ refspecs: ['main', 'dev:rel'], tags: true, followTags: true, dryRun: true, setUpstream: true, repair: true })
  })

  it('overwrite and delete are risky, and only a confirmed push carries confirmed', () => {
    const f = emptyForm()
    f.push.overwrite = 'lease'
    expect(pushIsRisky(f.push)).toBe(true)
    expect(transferOpts('push', f)).toEqual({ overwrite: 'lease' })
    expect(transferOpts('push', f, true)).toEqual({ overwrite: 'lease', confirmed: true })
    f.push.overwrite = 'none'
    Object.assign(f.push, { delete: true, refspecs: 'old', tags: true })
    expect(pushIsRisky(f.push)).toBe(true)
    expect(transferOpts('push', f, true)).toEqual({ refspecs: ['old'], delete: true, confirmed: true })
    // a safe push never claims confirmation
    expect(transferOpts('push', emptyForm(), true)).toEqual({})
  })

  it('parses refspecs from spaces, commas and lines', () => {
    expect(parseRefspecs(' a,b  c\nd:e ')).toEqual(['a', 'b', 'c', 'd:e'])
  })

  it('the sync store passes the form to the API', async () => {
    await open()
    const s = useSyncStore()
    await s.refresh()
    api.push.mockResolvedValue({ result: 'pushed', objects_sent: 0, bytes_uploaded: 0 })
    s.form.push.dryRun = true
    await s.run('push')
    expect(api.push).toHaveBeenCalledWith('/repos/hero', 'origin', expect.any(String), expect.any(Function), { dryRun: true })
    s.form.push.overwrite = 'force'
    expect(s.pushNeedsConfirm).toBe(true)
    await s.run('push', true)
    expect(api.push).toHaveBeenLastCalledWith('/repos/hero', 'origin', expect.any(String), expect.any(Function), { dryRun: true, overwrite: 'force', confirmed: true })
  })

  it('rename and change-address re-read the remotes; the token travels as an option', async () => {
    await open()
    const s = useSyncStore()
    await s.refresh()
    api.remoteRename.mockResolvedValue({})
    api.remoteSetUrl.mockResolvedValue({})
    api.remoteAdd.mockResolvedValue({ remotes: [] })
    expect(await s.renameRemote('origin', 'main-remote')).toBe(true)
    expect(api.remoteRename).toHaveBeenCalledWith('/repos/hero', 'origin', 'main-remote')
    expect(await s.setRemoteUrl('origin', 'https://x.example/r', true)).toBe(true)
    expect(api.remoteSetUrl).toHaveBeenCalledWith('/repos/hero', 'origin', 'https://x.example/r', true)
    await s.addRemote('b', 'https://y.example/r', { fetch: true, token: 'tok' })
    expect(api.remoteAdd).toHaveBeenCalledWith('/repos/hero', 'b', 'https://y.example/r', { fetch: true, token: 'tok' })
  })

  it('a failed add re-reads the remotes: the fetch can fail after the remote was saved', async () => {
    await open()
    const s = useSyncStore()
    await s.refresh()
    api.remoteAdd.mockRejectedValue({ code: 'error', message: "remote 'b' was added, but the fetch failed" })
    api.remoteList.mockResolvedValue({ remotes: [{ name: 'b', url: 'https://y.example/r' }] })
    expect(await s.addRemote('b', 'https://y.example/r', { fetch: true })).toBe(false)
    await vi.waitFor(() => expect(s.remotes.map((r) => r.name)).toEqual(['b']))
    expect(s.actionError?.message).toContain('was added')
  })
})

describe('locks remote and owner', () => {
  it('the chosen remote rides on list, lock and unlock; owner only on lock', async () => {
    await open()
    const l = useLocksStore()
    l.remote = 'backup'
    await l.refresh()
    expect(api.lockList).toHaveBeenCalledWith('/repos/hero', 'backup')
    api.lockCreate.mockResolvedValue({ owner: 'ana' })
    await l.lock('a.psd', ' ana ')
    expect(api.lockCreate).toHaveBeenCalledWith('/repos/hero', 'a.psd', 'ana', 'backup')
    api.lockUnlock.mockResolvedValue({})
    await l.unlock({ lock_id: 'x', path: 'a.psd', owner: 'ana', created_at: 1 } as never, false)
    expect(api.lockUnlock).toHaveBeenCalledWith('/repos/hero', 'x', false, 'backup')
  })
})

describe('changes options', () => {
  it('stage options reach add; a preview stages nothing and says so', async () => {
    await open()
    const c = useChangesStore()
    api.add.mockResolvedValue({ added: 2, modified: 0, deleted: 1, bytes: 0 })
    c.addOpts.force = true
    await c.stageAll()
    expect(api.add).toHaveBeenCalledWith('/repos/hero', ['.'], expect.objectContaining({ force: true, dryRun: false }), expect.any(Function), expect.any(String))
    c.addOpts.dryRun = true
    await c.stageAll()
    expect(c.preview).toContain('3 files would be staged')
  })

  it('save options reach commit; a message file replaces the message; preview keeps the message', async () => {
    await open()
    const c = useChangesStore()
    expect(c.canSave).toBe(true)
    // I1: a stopped operation finishes through its own step, never a plain save.
    useRepoStore().status = { ...useRepoStore().status, operation: { kind: 'merge', ours: 'main', ours_commit: null, theirs: 'x' } } as never
    expect(c.canSave).toBe(false)
    useRepoStore().status = { ...useRepoStore().status, operation: undefined } as never
    expect(c.canSave).toBe(true)
    c.message = 'hello'
    c.commitForm.signoff = true
    c.commitForm.include = 'a.psd\r\n\nb.psd'
    c.commitForm.dryRun = true
    api.commit.mockResolvedValue({ would_commit: true, staged: 1, deleted: 0, staged_paths: [], summary: 'hello' })
    await c.commit()
    expect(api.commit).toHaveBeenCalledWith('/repos/hero', 'hello', { include: ['a.psd', 'b.psd'], signoff: true, dryRun: true })
    expect(c.message).toBe('hello')
    expect(c.preview).toContain('would save 1 file, 0 deletions')
    c.message = ''
    expect(c.needsMessage).toBe(true)
    c.commitForm.messageFile = 'C:\\m.txt'
    expect(c.needsMessage).toBe(false)
  })

  it('BUG-013: one-time save options reset after a real save, and stay after a preview', async () => {
    await open()
    const c = useChangesStore()
    c.message = 'one'
    Object.assign(c.commitForm, { author: 'Ana <a@x.io>', date: '2026-01-15T10:30:00Z', signoff: true, allowEmpty: true, all: true, include: 'a.psd', messageFile: 'C:\m.txt' })
    c.commitForm.dryRun = true
    api.commit.mockResolvedValue({ would_commit: true, staged: 1, deleted: 0, staged_paths: [], summary: 'one' })
    await c.commit()
    expect(c.commitForm.author).toBe('Ana <a@x.io>')
    c.commitForm.dryRun = false
    await c.commit()
    expect(c.commitForm).toEqual({ messageFile: '', all: false, include: '', author: '', date: '', allowEmpty: false, signoff: false, dryRun: false })
    c.message = 'two'
    api.commit.mockClear()
    await c.commit()
    expect(api.commit).toHaveBeenCalledWith('/repos/hero', 'two', {})
  })

  it('BUG-038: staging can be cancelled; a cancel is not an error and the files stay as they were', async () => {
    await open()
    const c = useChangesStore()
    const ops = useOpsStore()
    let opId = ''
    api.add.mockImplementation((_r: string, _p: string[], _o: unknown, _cb: unknown, id: string) => {
      opId = id
      return new Promise((_res, rej) => {
        api.cancelOp.mockImplementation(async () => rej({ code: 'cancelled', message: 'Cancelled' }))
      })
    })
    const staging = c.stageAll()
    await vi.waitFor(() => expect(ops.running.length).toBe(1))
    expect(opId).not.toBe('')
    await ops.cancel(ops.running[0]!.id)
    await staging
    expect(api.cancelOp).toHaveBeenCalledWith(opId)
    expect(c.error).toBeNull()
    expect(c.busy).toBeNull()
    expect(ops.finished[0]!.status).toBe('cancelled')
    expect(c.preview).toContain('Stopped')
  })

  it('BUG-038: a stop after some batches says how many files were already included', async () => {
    await open()
    const c = useChangesStore()
    const ops = useOpsStore()
    const paths = Array.from({ length: 250 }, (_, i) => `f${i}.psd`)
    api.add.mockResolvedValueOnce({ added: 200, modified: 0, deleted: 0 })
    api.add.mockImplementationOnce(() => new Promise((_res, rej) => {
      api.cancelOp.mockImplementation(async () => rej({ code: 'cancelled', message: 'Cancelled' }))
    }))
    const staging = c.setIncluded(paths, true)
    await vi.waitFor(() => expect(api.add).toHaveBeenCalledTimes(2))
    await ops.cancel(ops.running[0]!.id)
    await staging
    expect(c.preview).toBe('Stopped. 200 of 250 files were included. You can include the rest later.')
  })

  it('unstage sends paths; Unstage all sends a dot', async () => {
    await open()
    const c = useChangesStore()
    api.unstage.mockResolvedValue({})
    await c.unstageAll()
    expect(api.unstage).toHaveBeenCalledWith('/repos/hero', ['.'])
    c.select('a.psd')
    await c.excludeSelected()
    expect(api.unstage).toHaveBeenLastCalledWith('/repos/hero', ['a.psd'])
  })
})
