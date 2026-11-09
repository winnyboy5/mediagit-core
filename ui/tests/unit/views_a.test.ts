import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  add: vi.fn(),
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
    expect(api.add).toHaveBeenCalledWith('/repos/hero', ['.'], expect.objectContaining({ force: true, dryRun: false }), expect.any(Function))
    c.addOpts.dryRun = true
    await c.stageAll()
    expect(c.preview).toContain('3 files would be staged')
  })

  it('save options reach commit; a message file replaces the message; preview keeps the message', async () => {
    await open()
    const c = useChangesStore()
    expect(c.canSave).toBe(true)
    c.message = 'hello'
    c.commitForm.signoff = true
    c.commitForm.include = 'a.psd\r\n\nb.psd'
    c.commitForm.dryRun = true
    api.commit.mockResolvedValue({ would_commit: true, staged: 1, deleted: 0, staged_paths: [], summary: 'hello' })
    await c.commit()
    expect(api.commit).toHaveBeenCalledWith('/repos/hero', 'hello', { include: ['a.psd', 'b.psd'], signoff: true, dryRun: true })
    expect(c.message).toBe('hello')
    expect(c.preview).toContain('would save 1 files')
    c.message = ''
    expect(c.needsMessage).toBe(true)
    c.commitForm.messageFile = 'C:\\m.txt'
    expect(c.needsMessage).toBe(false)
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
