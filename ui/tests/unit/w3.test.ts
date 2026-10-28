import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import type { LockReport } from '@/api'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  branchList: vi.fn(),
  log: vi.fn(),
  stats: vi.fn(),
  gc: vi.fn(),
  fsck: vi.fn(),
  verify: vi.fn(),
  authLogin: vi.fn(),
  authRun: vi.fn(),
  keyStatus: vi.fn(),
  keyInit: vi.fn(),
  keyRecover: vi.fn(),
  configList: vi.fn(),
  configSet: vi.fn(),
  configUnset: vi.fn(),
  sparseList: vi.fn(),
  sparseSet: vi.fn(),
  sparseDisable: vi.fn(),
  rebaseStart: vi.fn(),
  pickStart: vi.fn(),
  sequencerControl: vi.fn(),
  mergeResolve: vi.fn(),
  bisectStart: vi.fn(),
  bisectMark: vi.fn(),
  bisectReset: vi.fn(),
  reflog: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { stepSummary, useBisectStore, usePickStore, useRebaseStore, useReflogStore } from '@/stores/advanced'
import { useIdentityStore } from '@/stores/identity'
import { filterLocks } from '@/stores/locks'
import { useMaintenanceStore } from '@/stores/maintenance'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSecurityStore } from '@/stores/security'
import { applyLook, useLookStore, useRepoSettingsStore } from '@/stores/settings'
import { useStorageStore } from '@/stores/storage'
import { useUnlockStore } from '@/stores/unlock'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  localStorage.clear()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' }, summary: {} })
  api.allowRepo.mockResolvedValue(undefined)
  api.branchList.mockResolvedValue({ current: 'main', branches: [] })
  api.log.mockResolvedValue({ commits: [], refs: [] })
  api.authRun.mockResolvedValue({ identity: null })
  api.keyStatus.mockResolvedValue({ enabled: false })
  api.configList.mockResolvedValue({ entries: [] })
  api.sparseList.mockResolvedValue({ enabled: false, patterns: [] })
})

const open = () => {
  const repo = useRepoStore()
  repo.path = '/repos/hero'
  return repo
}
const needsInput = { code: 'needs_input', message: 'passphrase' }

describe('unlock store', () => {
  it('runs once when nothing is needed', async () => {
    const u = useUnlockStore()
    const fn = vi.fn().mockResolvedValue(7)
    expect(await u.withUnlock(fn)).toBe(7)
    expect(fn).toHaveBeenCalledWith(undefined)
    expect(u.open).toBe(false)
  })

  it('asks on needs_input, retries with the answer, and does not keep it', async () => {
    const u = useUnlockStore()
    const fn = vi.fn().mockRejectedValueOnce(needsInput).mockResolvedValueOnce('ok')
    const run = u.withUnlock(fn)
    await vi.waitFor(() => expect(u.open).toBe(true))
    expect(u.retry).toBe(false)
    u.answer('pw1')
    expect(await run).toBe('ok')
    expect(fn.mock.calls).toEqual([[undefined], ['pw1']])
    expect(u.open).toBe(false)
    expect(JSON.stringify(u.$state)).not.toContain('pw1')
  })

  it('asks again, marked as a retry, when the answer was wrong', async () => {
    const u = useUnlockStore()
    const fn = vi.fn().mockRejectedValueOnce(needsInput).mockRejectedValueOnce(needsInput).mockResolvedValueOnce('ok')
    const run = u.withUnlock(fn)
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('bad')
    await vi.waitFor(() => expect(u.retry).toBe(true))
    u.answer('good')
    expect(await run).toBe('ok')
  })

  it('cancelling rejects with `cancelled`; other errors pass straight through', async () => {
    const u = useUnlockStore()
    const run = u.withUnlock(vi.fn().mockRejectedValue(needsInput))
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer(null)
    await expect(run).rejects.toMatchObject({ code: 'cancelled' })
    await expect(u.withUnlock(vi.fn().mockRejectedValue({ code: 'error', message: 'x' }))).rejects.toMatchObject({ code: 'error' })
    expect(u.open).toBe(false)
  })
})

describe('storage and maintenance stores', () => {
  it('stats go through the unlock retry', async () => {
    open()
    const st = useStorageStore()
    api.stats.mockRejectedValueOnce(needsInput).mockResolvedValueOnce({ total_bytes: 5 })
    const p = st.refresh()
    await vi.waitFor(() => expect(useUnlockStore().open).toBe(true))
    useUnlockStore().answer('pw')
    await p
    expect(api.stats.mock.calls).toEqual([['/repos/hero', undefined], ['/repos/hero', 'pw']])
    expect(st.stats?.total_bytes).toBe(5)
  })

  it('a preview gc is unconfirmed; a real one carries confirmed and stamps the last-run time', async () => {
    open()
    const m = useMaintenanceStore()
    api.gc.mockResolvedValue({ errors: [], objects_deleted: 3 })
    await m.runGc(true, false)
    expect(api.gc).toHaveBeenLastCalledWith('/repos/hero', true, false, undefined, expect.any(Function))
    expect(m.gc?.preview).toBe(true)
    await m.runGc(false, true)
    expect(api.gc).toHaveBeenLastCalledWith('/repos/hero', false, true, undefined, expect.any(Function))
    expect(m.gc?.preview).toBe(false)
    expect(m.last['/repos/hero']?.gc).toBeGreaterThan(0)
    expect(JSON.parse(localStorage.getItem('mg.maint.last')!)['/repos/hero'].gc).toBeGreaterThan(0)
  })

  it('fsck and verify keep their reports apart; findings raise a warning, not a failure', async () => {
    open()
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue({ healthy: false, objects_checked: 9, issues: [] })
    api.verify.mockResolvedValue({ healthy: true, objects_checked: 9, issues: [] })
    expect(await m.runFsck(true)).toBe(true)
    expect(api.fsck).toHaveBeenCalledWith('/repos/hero', true, undefined, expect.any(Function))
    expect(await m.runVerify(false)).toBe(true)
    expect(m.fsck?.report.healthy).toBe(false)
    expect(m.verify?.report.healthy).toBe(true)
    expect(useOpsStore().finished.map((o) => o.label)).toEqual(expect.arrayContaining(['Deep integrity check', 'Verify repository']))
  })

  it('runs one task at a time', async () => {
    open()
    const m = useMaintenanceStore()
    let release!: (v: unknown) => void
    api.fsck.mockImplementation(() => new Promise((r) => (release = r)))
    const first = m.runFsck(false)
    await nextTick()
    expect(m.busy).toBe(true)
    expect(await m.runVerify(true)).toBe(false)
    release({ healthy: true })
    await first
  })
})

describe('security store: secrets', () => {
  it('passes the password on without storing it or giving the op a way to re-run it', async () => {
    open()
    const sec = useSecurityStore()
    api.authLogin.mockRejectedValue({ code: 'error', message: 'Wrong password.' })
    expect(await sec.login('', 'mira', 'hunter2-secret')).toBeNull()
    expect(api.authLogin).toHaveBeenCalledWith('/repos/hero', null, 'mira', 'hunter2-secret')
    const op = useOpsStore().ops.at(-1)!
    expect(op.status).toBe('failed')
    expect(op.retry).toBeUndefined()
    expect(JSON.stringify(sec.$state) + JSON.stringify(useOpsStore().ops.map((o) => [o.label, o.summary, o.error, o.phase]))).not.toContain('hunter2')
  })

  it('keeps the recovery code in memory only until dismissed, never in ops or storage', async () => {
    open()
    const sec = useSecurityStore()
    api.keyInit.mockResolvedValue({ action: 'init', enabled: true, recovery_code: 'RC-SECRET-CODE' })
    expect(await sec.enableEncryption('')).toBe(true)
    expect(api.keyInit).toHaveBeenCalledWith('/repos/hero', null)
    expect(sec.recoveryCode).toBe('RC-SECRET-CODE')
    const ops = JSON.stringify(useOpsStore().ops.map((o) => [o.label, o.summary, o.error]))
    expect(ops).not.toContain('SECRET-CODE')
    expect(useOpsStore().ops.at(-1)!.retry).toBeUndefined()
    expect(JSON.stringify({ ...localStorage })).not.toContain('SECRET-CODE')
    sec.dismissRecovery()
    expect(sec.recoveryCode).toBeNull()
  })

  it('a failed init sets no recovery code; changing repository clears one', async () => {
    const repo = open()
    const sec = useSecurityStore()
    api.keyInit.mockRejectedValue({ code: 'error', message: 'already' })
    expect(await sec.enableEncryption('pw')).toBe(false)
    expect(sec.recoveryCode).toBeNull()
    api.keyInit.mockResolvedValue({ recovery_code: 'RC-1' })
    await sec.enableEncryption('')
    repo.path = '/repos/other'
    await nextTick()
    expect(sec.recoveryCode).toBeNull()
  })

  it('recover sends the code and an optional passphrase; blank becomes null', async () => {
    open()
    const sec = useSecurityStore()
    api.keyRecover.mockResolvedValue({ enabled: true })
    await sec.recover('CODE-1', '')
    expect(api.keyRecover).toHaveBeenCalledWith('/repos/hero', 'CODE-1', null)
    expect(useOpsStore().ops.at(-1)!.retry).toBeUndefined()
  })

  it('reads account and key independently', async () => {
    open()
    const sec = useSecurityStore()
    api.authRun.mockRejectedValue({ code: 'error', message: 'no server' })
    api.keyStatus.mockResolvedValue({ enabled: true, source: 'OS keychain' })
    await sec.refresh()
    expect(sec.authError?.message).toBe('no server')
    expect(sec.key?.source).toBe('OS keychain')
  })
})

describe('identity and locks', () => {
  it('collects the account and author names; either marks a lock as yours', async () => {
    open()
    const id = useIdentityStore()
    api.authRun.mockResolvedValue({ identity: { username: 'mira' } })
    api.configList.mockResolvedValue({ entries: [{ key: 'author.name', value: 'Mira Okafor' }, { key: 'author.email', value: 'm@x.example' }] })
    await id.refresh()
    expect(id.names).toEqual(['mira', 'Mira Okafor'])
    const locks = ['mira', 'Mira Okafor', 'Dev'].map((owner) => ({ lock_id: owner, path: owner, owner, created_at: 1 }) as LockReport)
    expect(filterLocks(locks, 'mine', id.names).map((l) => l.owner)).toEqual(['mira', 'Mira Okafor'])
    expect(filterLocks(locks, 'others', id.names).map((l) => l.owner)).toEqual(['Dev'])
    expect(filterLocks(locks, 'mine', 'Dev').map((l) => l.owner)).toEqual(['Dev'])
    expect(filterLocks(locks, 'mine', null)).toEqual([])
  })

  it('tolerates no account and no author', async () => {
    open()
    const id = useIdentityStore()
    api.authRun.mockRejectedValue({ code: 'error', message: 'x' })
    api.configList.mockRejectedValue({ code: 'error', message: 'y' })
    await id.refresh()
    expect(id.names).toEqual([])
    expect(id.loaded).toBe(true)
  })
})

describe('rebase and cherry-pick stores', () => {
  const conflict = { action: 'start', status: 'conflict', head: null, commits_rebased: null, conflict_commit: 'cc', conflicts: ['a.psd', 'b.fbx'] }

  it('a clash becomes a per-file list; "theirs" is the commit being replayed', async () => {
    open()
    const rb = useRebaseStore()
    api.rebaseStart.mockResolvedValue(conflict)
    await rb.start('feature')
    expect(api.rebaseStart).toHaveBeenCalledWith('/repos/hero', 'feature', expect.any(Function))
    expect(rb.active).toBe(true)
    expect(rb.clashes.map((c) => c.path)).toEqual(['a.psd', 'b.fbx'])
    api.mergeResolve.mockResolvedValue(null)
    await rb.resolve('a.psd', 'theirs')
    expect(api.mergeResolve).toHaveBeenCalledWith('/repos/hero', 'a.psd', 'theirs', 'cc')
    expect(rb.canContinue).toBe(false)
    await rb.resolve('b.fbx', 'ours')
    expect(rb.canContinue).toBe(true)
    api.sequencerControl.mockResolvedValue({ action: 'continue', status: 'rebased', head: 'h', commits_rebased: 2, conflict_commit: null, conflicts: [] })
    await rb.control('continue')
    expect(api.sequencerControl).toHaveBeenCalledWith('/repos/hero', 'rebase', 'continue', expect.any(Function))
    expect(rb.active).toBe(false)
    expect(rb.last?.status).toBe('rebased')
  })

  it('cherry-pick uses its own command name and is independent of rebase', async () => {
    open()
    const pick = usePickStore()
    const rb = useRebaseStore()
    api.pickStart.mockResolvedValue({ action: 'start', status: 'conflict', commits: [], conflict_commit: 'dd', conflicts: ['x.wav'] })
    await pick.start('abc')
    expect(api.pickStart).toHaveBeenCalledWith('/repos/hero', ['abc'], expect.any(Function))
    expect(pick.active).toBe(true)
    expect(rb.active).toBe(false)
    api.sequencerControl.mockResolvedValue({ action: 'abort', status: 'aborted', commits: [], conflict_commit: null, conflicts: [] })
    await pick.control('abort')
    expect(api.sequencerControl).toHaveBeenCalledWith('/repos/hero', 'cherry-pick', 'abort', expect.any(Function))
    expect(pick.active).toBe(false)
  })

  it('cannot resolve without a commit to take "theirs" from', async () => {
    open()
    expect(await useRebaseStore().resolve('a', 'ours')).toBe(false)
  })

  it('summarises each outcome', () => {
    expect(stepSummary({ status: 'conflict', conflicts: ['a'], conflictCommit: 'x' })).toBe('Stopped: 1 file clashes')
    expect(stepSummary({ status: 'conflict', conflicts: ['a', 'b'], conflictCommit: 'x' })).toBe('Stopped: 2 files clash')
    expect(stepSummary({ status: 'rebased', conflicts: [], conflictCommit: null })).toBe('Rebased')
    expect(stepSummary({ status: 'weird', conflicts: [], conflictCommit: null })).toBe('weird')
  })
})

describe('bisect and reflog stores', () => {
  it('follows the report: active until complete is read as a result, inactive after reset', async () => {
    open()
    const b = useBisectStore()
    expect(b.active).toBe(false)
    api.bisectStart.mockResolvedValue({ status: 'testing', current: 'c1', remaining_steps: 3, good: [], skipped: [], log: [] })
    await b.start('HEAD', 'old')
    expect(api.bisectStart).toHaveBeenCalledWith('/repos/hero', 'HEAD', 'old', expect.any(Function))
    expect(b.active).toBe(true)
    api.bisectMark.mockResolvedValue({ status: 'complete', first_bad: 'c1', good: [], skipped: [], log: [] })
    await b.mark('bad')
    expect(api.bisectMark).toHaveBeenCalledWith('/repos/hero', 'bad', null, expect.any(Function))
    expect(b.report?.first_bad).toBe('c1')
    api.bisectReset.mockResolvedValue({ status: 'reset', good: [], skipped: [], log: [] })
    await b.reset()
    expect(b.active).toBe(false)
  })

  it('keeps the error from a failed step and the last good report', async () => {
    open()
    const b = useBisectStore()
    api.bisectStart.mockResolvedValue({ status: 'testing', good: [], skipped: [], log: [] })
    await b.start('HEAD', null)
    api.bisectMark.mockRejectedValue({ code: 'error', message: 'oops' })
    expect(await b.mark('good')).toBe(false)
    expect(b.error?.message).toBe('oops')
    expect(b.report?.status).toBe('testing')
  })

  it('reads a reflog for the chosen ref and resets on repository change', async () => {
    const repo = open()
    const r = useReflogStore()
    api.reflog.mockResolvedValue({ reference: 'HEAD', entries: [{ index: 0 }] })
    await r.refresh()
    expect(api.reflog).toHaveBeenCalledWith('/repos/hero', 'HEAD', 100)
    r.reference = 'main'
    await r.refresh()
    expect(api.reflog).toHaveBeenLastCalledWith('/repos/hero', 'main', 100)
    repo.path = '/repos/other'
    await nextTick()
    expect(r.entries).toEqual([])
    expect(r.reference).toBe('HEAD')
  })
})

describe('look and repository settings', () => {
  it('applies theme and density to the page and remembers them', async () => {
    const root = document.createElement('div')
    applyLook('dark', 'compact', root)
    expect(root.getAttribute('data-theme')).toBe('dark')
    expect(root.getAttribute('data-density')).toBe('compact')
    applyLook('system', 'comfortable', root)
    expect(root.hasAttribute('data-theme')).toBe(false)
    expect(root.hasAttribute('data-density')).toBe(false)

    const look = useLookStore()
    expect(look.theme).toBe('system')
    look.theme = 'light'
    look.density = 'compact'
    await nextTick()
    expect(localStorage.getItem('mg.theme')).toBe('"light"')
    expect(localStorage.getItem('mg.density')).toBe('"compact"')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
    look.theme = 'system'
    look.density = 'comfortable'
    await nextTick()
  })

  it('starts from what was saved', () => {
    localStorage.setItem('mg.theme', '"dark"')
    localStorage.setItem('mg.density', '"compact"')
    setActivePinia(createPinia())
    const look = useLookStore()
    expect(look.theme).toBe('dark')
    expect(look.density).toBe('compact')
    look.theme = 'system'
    look.density = 'comfortable'
  })

  it('saves author name then email, and reads values back', async () => {
    open()
    const s = useRepoSettingsStore()
    api.configList.mockResolvedValue({ entries: [{ key: 'author.name', value: 'Mira' }, { key: 'author.email', value: null }] })
    api.sparseList.mockResolvedValue({ enabled: true, mode: 'cone', patterns: ['art'] })
    await s.refresh()
    expect(s.value('author.name')).toBe('Mira')
    expect(s.value('author.email')).toBeNull()
    expect(s.sparse?.patterns).toEqual(['art'])
    api.configSet.mockResolvedValue({ entries: [] })
    await s.setAuthor('Mira O.', 'm@x.example')
    expect(api.configSet.mock.calls.map((c) => c.slice(1))).toEqual([['author.name', 'Mira O.'], ['author.email', 'm@x.example']])
    expect(api.configList.mock.calls.length).toBeGreaterThan(1)
  })

  it('stops after a failed name save, and sparse set / disable call their commands', async () => {
    open()
    const s = useRepoSettingsStore()
    api.configSet.mockRejectedValueOnce({ code: 'error', message: 'bad' })
    await s.setAuthor('X', 'x@y.zz')
    expect(api.configSet).toHaveBeenCalledTimes(1)
    expect(s.actionError?.message).toBe('bad')
    api.sparseSet.mockResolvedValue({ files_added: 1, files_removed: 2 })
    api.sparseDisable.mockResolvedValue({ files_added: 5 })
    expect(await s.applySparse(['art'], true)).toBe(true)
    expect(api.sparseSet).toHaveBeenCalledWith('/repos/hero', ['art'], true, expect.any(Function))
    expect(await s.disableSparse()).toBe(true)
    expect(api.sparseDisable).toHaveBeenCalledWith('/repos/hero', expect.any(Function))
  })
})
