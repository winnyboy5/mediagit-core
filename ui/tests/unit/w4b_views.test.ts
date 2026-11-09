import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  tagList: vi.fn(),
  tagCreate: vi.fn(),
  tagDelete: vi.fn(),
  tagShow: vi.fn(),
  tagVerify: vi.fn(),
  stashList: vi.fn(),
  stashShow: vi.fn(),
  stashSave: vi.fn(),
  stashApply: vi.fn(),
  stashPop: vi.fn(),
  gc: vi.fn(),
  fsck: vi.fn(),
  verify: vi.fn(),
  authRegister: vi.fn(),
  authPasswd: vi.fn(),
  keyRotateMaster: vi.fn(),
  authRun: vi.fn(),
  keyStatus: vi.fn(),
  configList: vi.fn(),
  configSchema: vi.fn(),
  configSet: vi.fn(),
  configUnset: vi.fn(),
  sparseList: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { useToast } from '@/components/useToast'
import { useMaintenanceStore } from '@/stores/maintenance'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSecurityStore } from '@/stores/security'
import { useRepoSettingsStore } from '@/stores/settings'
import { useTagsStashStore } from '@/stores/tagsStash'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' }, summary: { staged: 0, modified: 0, deleted: 0, untracked: 0 } })
  api.allowRepo.mockResolvedValue(undefined)
  api.tagList.mockResolvedValue({ tags: [] })
  api.stashList.mockResolvedValue({ stashes: [] })
  api.authRun.mockResolvedValue(null)
  api.keyStatus.mockResolvedValue(null)
})

const open = () => {
  useRepoStore().path = '/repos/hero'
}

describe('tags and stash: options', () => {
  it('passes the tag options through, trimming the note, and says when a tag was replaced', async () => {
    open()
    const s = useTagsStashStore()
    api.tagCreate.mockResolvedValue({ action: 'create', created: null, deleted: [], missing: [] })
    await s.createTag('v1', { commit: 'abc1234', message: ' Final ', tagger: 'Mira', email: 'm@x.io' })
    expect(api.tagCreate).toHaveBeenLastCalledWith('/repos/hero', 'v1', { commit: 'abc1234', message: 'Final', tagger: 'Mira', email: 'm@x.io' })
    await s.createTag('v1', { force: true, confirmed: true })
    expect(api.tagCreate).toHaveBeenLastCalledWith('/repos/hero', 'v1', { force: true, confirmed: true, message: null })
    expect(useOpsStore().finished.map((o) => o.summary)).toEqual(expect.arrayContaining(['Tagged abc1234'.replace('abc1234', 'v1'), 'Replaced tag v1']))
  })

  it('the tag filter narrows the list through tag list PATTERN and clears with the repository', async () => {
    open()
    const s = useTagsStashStore()
    await s.refresh()
    expect(api.tagList).toHaveBeenLastCalledWith('/repos/hero', null)
    s.filter = ' v1.* '
    await s.refresh()
    expect(api.tagList).toHaveBeenLastCalledWith('/repos/hero', 'v1.*')
    useRepoStore().path = '/repos/other'
    await vi.waitFor(() => expect(s.filter).toBe(''))
  })

  it('shows a tag and checks its signature; an invalid signature is an error, not a report', async () => {
    open()
    const s = useTagsStashStore()
    api.tagShow.mockResolvedValue({ name: 'v1', oid: 'aa', annotated: true, message: 'm', tagger_name: 'Mira', tagger_email: null, date: null, signed: true })
    await s.showTag('v1')
    expect(s.detail?.show?.tagger_name).toBe('Mira')
    api.tagVerify.mockResolvedValue({ name: 'v1', oid: 'aa', annotated: true, signed: true, fingerprint: 'F1' })
    await s.verifyTag()
    expect(s.detail?.verify?.fingerprint).toBe('F1')
    api.tagVerify.mockRejectedValue({ code: 'error', message: 'signature does not match' })
    await s.verifyTag()
    expect(s.detail?.verify).toBeNull()
    expect(s.detail?.verifyError?.message).toContain('does not match')
    s.closeTag()
    expect(s.detail).toBeNull()
  })

  it('a tag that cannot be shown keeps its error', async () => {
    open()
    const s = useTagsStashStore()
    api.tagShow.mockRejectedValue({ code: 'error', message: 'no such tag' })
    await s.showTag('gone')
    expect(s.detail?.error?.message).toBe('no such tag')
  })

  it('shelving selected files passes the paths; with none it stashes everything', async () => {
    open()
    const s = useTagsStashStore()
    api.stashSave.mockResolvedValue({ oid: 'a' })
    await s.save(' WIP ', false, ['a.txt', 'art/b.psd'])
    expect(api.stashSave).toHaveBeenLastCalledWith('/repos/hero', 'WIP', false, ['a.txt', 'art/b.psd'], expect.any(Function))
    await s.save('', true)
    expect(api.stashSave).toHaveBeenLastCalledWith('/repos/hero', null, true, [], expect.any(Function))
    const labels = useOpsStore().finished.map((o) => o.label)
    expect(labels).toEqual(expect.arrayContaining(['Shelve selected files', 'Stash changes']))
  })

  it('apply and pop carry --index and --force', async () => {
    open()
    const s = useTagsStashStore()
    api.stashApply.mockResolvedValue({ files_updated: 2 })
    api.stashPop.mockResolvedValue({ files_updated: 1 })
    await s.apply(1, { index: true })
    expect(api.stashApply).toHaveBeenLastCalledWith('/repos/hero', 1, { index: true }, expect.any(Function))
    await s.pop(0, { index: true, force: true })
    expect(api.stashPop).toHaveBeenLastCalledWith('/repos/hero', 0, { index: true, force: true }, expect.any(Function))
  })

  it('reads one stash entry', async () => {
    open()
    const s = useTagsStashStore()
    api.stashShow.mockResolvedValue({ stashes: [{ index: 2, oid: 'x', message: 'WIP', branch: 'main', timestamp: 't' }] })
    expect((await s.showStash(2))?.message).toBe('WIP')
    api.stashShow.mockRejectedValue({ code: 'error', message: 'no stash 9' })
    expect(await s.showStash(9)).toBeNull()
    expect(s.actionError?.message).toBe('no stash 9')
  })
})

describe('maintenance: options', () => {
  it('gc passes its options', async () => {
    open()
    const m = useMaintenanceStore()
    api.gc.mockResolvedValue({ errors: [], objects_deleted: 0 })
    await m.runGc(true, false, { repack: true, noPrune: true })
    expect(api.gc).toHaveBeenLastCalledWith('/repos/hero', true, false, { repack: true, noPrune: true }, undefined, expect.any(Function))
  })

  it('a repair is labelled, remembered as a preview or a real run, and carries its confirmation', async () => {
    open()
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue({ healthy: true, objects_checked: 3, repaired: 2, issues: [] })
    await m.runFsck({ repair: true, dryRun: true })
    expect(m.fsck?.repair).toBe('preview')
    await m.runFsck({ repair: true, confirmed: true })
    expect(api.fsck).toHaveBeenLastCalledWith('/repos/hero', { repair: true, confirmed: true }, undefined, expect.any(Function))
    expect(m.fsck?.repair).toBe('done')
    await m.runFsck({ deep: true, all: true })
    expect(m.fsck?.repair).toBeUndefined()
    expect(useOpsStore().finished.map((o) => o.label)).toEqual(expect.arrayContaining(['Preview repair', 'Repair repository', 'Deep integrity check']))
  })

  it('verify of one version or a range reports versions; the whole repository reports objects', async () => {
    open()
    const m = useMaintenanceStore()
    api.verify.mockResolvedValue({ start: null, end: 'abc', commits: 3, verified: 3, errors: 0, problems: [] })
    await m.runVerify({ commit: 'main', detailed: true })
    expect(api.verify).toHaveBeenLastCalledWith('/repos/hero', { commit: 'main', detailed: true }, undefined, expect.any(Function))
    expect(m.verifyRange?.report.verified).toBe(3)
    expect(m.verify).toBeNull()
    api.verify.mockResolvedValue({ healthy: true, objects_checked: 9, refs_checked: 1, corrupted_objects: 0, broken_refs: 0, missing_objects: 0, dangling_objects: 0, repaired: null, issues: [] })
    await m.runVerify({ checksums: true })
    expect(m.verify?.report.objects_checked).toBe(9)
    expect(m.verifyRange).toBeNull()
  })

  it('a range with errors is a warning', async () => {
    open()
    const before = useToast().items.value.length
    const m = useMaintenanceStore()
    api.verify.mockResolvedValue({ start: 'a', end: 'b', commits: 2, verified: 1, errors: 1, problems: ['bad object'] })
    await m.runVerify({ start: 'a', end: 'b' })
    expect(useOpsStore().finished.at(-1)!.summary).toBe('Problems found')
    // The result card is on screen: Activity records it, no toast covers the problem list.
    expect(useOpsStore().finished.at(-1)!.quiet).toBe(true)
    expect(useToast().items.value.length).toBe(before)
  })
})

describe('settings: a form built from the CLI schema', () => {
  const spec = (key: string, type = 'string', secret = false) => ({ key, type, default: null, doc: key, enum_values: [], secret, min: null, max: null })
  const entry = (key: string, value: string | null, secret = false) => ({ key, value, secret })

  it('one field per schema key, one per existing name for a <name> key, author keys left to their own card', async () => {
    open()
    api.configSchema.mockResolvedValue({
      keys: [spec('author.name'), spec('storage.region'), spec('performance.pack_workers', 'int'), spec('remotes.<name>.url'), spec('remotes.<name>.token', 'string', true)],
    })
    api.configList.mockResolvedValue({
      entries: [
        entry('author.name', 'Mira'),
        entry('storage.region', 'eu-west-1'),
        entry('performance.pack_workers', null),
        entry('remotes.origin.url', 'https://s.example/x'),
        entry('remotes.origin.token', 'set', true),
        entry('remotes.backup.url', null),
        entry('remotes.backup.token', null, true),
      ],
    })
    api.sparseList.mockResolvedValue(null)
    const s = useRepoSettingsStore()
    await s.refresh()
    expect(s.fields.map((f) => f.key)).toEqual([
      'storage.region',
      'performance.pack_workers',
      'remotes.origin.url',
      'remotes.backup.url',
      'remotes.origin.token',
      'remotes.backup.token',
    ])
    expect(s.fields.find((f) => f.key === 'remotes.backup.url')?.name).toBe('backup')
    expect(s.fields.find((f) => f.key === 'storage.region')?.value).toBe('eu-west-1')
    expect(s.fields.find((f) => f.key === 'performance.pack_workers')?.value).toBeNull()
  })

  it('a secret goes to configSet flagged secret, with no retry and no value in the activity list', async () => {
    open()
    api.configList.mockResolvedValue({ entries: [] })
    api.configSchema.mockResolvedValue({ keys: [] })
    api.sparseList.mockResolvedValue(null)
    api.configSet.mockResolvedValue({ action: 'set', entries: [] })
    const s = useRepoSettingsStore()
    await s.setKey('remotes.origin.token', 'tok-SECRET-1', true)
    expect(api.configSet).toHaveBeenLastCalledWith('/repos/hero', 'remotes.origin.token', 'tok-SECRET-1', true)
    await s.setKey('storage.region', 'eu-west-1')
    expect(api.configSet).toHaveBeenLastCalledWith('/repos/hero', 'storage.region', 'eu-west-1', false)
    const ops = useOpsStore().ops
    expect(JSON.stringify(ops.map((o) => [o.label, o.summary, o.error]))).not.toContain('SECRET')
    expect(ops.find((o) => o.label === 'Save remotes.origin.token')?.retry).toBeUndefined()
  })

  it('without a readable schema there are no fields rather than stale ones', async () => {
    open()
    api.configList.mockResolvedValue({ entries: [entry('storage.region', 'x')] })
    api.configSchema.mockRejectedValue({ code: 'error', message: 'nope' })
    api.sparseList.mockResolvedValue(null)
    const s = useRepoSettingsStore()
    await s.refresh()
    expect(s.fields).toEqual([])
    expect(s.loaded).toBe(true)
  })
})

describe('security: secrets stay out of state and activity', () => {
  it('register, password change and key rotation pass their secrets on and keep nothing', async () => {
    open()
    const sec = useSecurityStore()
    api.authRegister.mockResolvedValue({})
    api.authPasswd.mockResolvedValue({})
    api.keyRotateMaster.mockResolvedValue({ action: 'rotate-master', enabled: true, source: 'a key file' })
    await sec.register('', 'mira', 'm@x.io', 'reg-secret-1')
    expect(api.authRegister).toHaveBeenCalledWith(null, 'mira', 'm@x.io', 'reg-secret-1')
    await sec.changePassword('https://s.example', 'old-secret-2', 'new-secret-3')
    expect(api.authPasswd).toHaveBeenCalledWith('https://s.example', 'old-secret-2', 'new-secret-3')
    await sec.rotateMaster('', 'cur-secret-4', 'next-secret-5')
    expect(api.keyRotateMaster).toHaveBeenCalledWith('/repos/hero', null, 'cur-secret-4', 'next-secret-5')
    await sec.rotateMaster(' C:/keys/new.key ', '', '')
    expect(api.keyRotateMaster).toHaveBeenLastCalledWith('/repos/hero', 'C:/keys/new.key', null, null)
    const seen = JSON.stringify(sec.$state) + JSON.stringify(useOpsStore().ops.map((o) => [o.label, o.summary, o.error, o.phase, o.retry === undefined]))
    for (const s of ['reg-secret', 'old-secret', 'new-secret', 'cur-secret', 'next-secret']) expect(seen).not.toContain(s)
    expect(useOpsStore().ops.every((o) => o.retry === undefined)).toBe(true)
  })

  it('a failed registration keeps its reason', async () => {
    open()
    const sec = useSecurityStore()
    api.authRegister.mockRejectedValue({ code: 'error', message: 'sign-up is closed on this server' })
    expect(await sec.register('', 'mira', 'm@x.io', 'pw-pw-pw')).toBeNull()
    expect(sec.actionError?.message).toContain('closed')
  })
})
