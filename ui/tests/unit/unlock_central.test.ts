import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: core.invoke,
  convertFileSrc: (p: string) => p,
  Channel: class {
    onmessage: unknown
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))

import { api, setStillLockedHandler, setUnlockHandler } from '@/api'
import { useUnlockStore } from '@/stores/unlock'

const needs = { code: 'needs_input', message: 'passphrase needed' }

beforeEach(() => {
  setActivePinia(createPinia())
  core.invoke.mockReset()
  setUnlockHandler(null)
  setStillLockedHandler(null)
})

describe('central unlock in the api layer', () => {
  it('a command that needs the passphrase runs again, once, after the repo is unlocked', async () => {
    const handler = vi.fn().mockResolvedValue(true)
    setUnlockHandler(handler)
    core.invoke.mockRejectedValueOnce(needs).mockResolvedValueOnce({ commits: [], refs: [] })
    await expect(api.log('/repos/a', 0, 10, false)).resolves.toEqual({ commits: [], refs: [] })
    expect(handler).toHaveBeenCalledWith('/repos/a')
    expect(core.invoke).toHaveBeenCalledTimes(2)
    // The retry is the same command with the same arguments: the passphrase is not among them.
    expect(core.invoke.mock.calls[1]).toEqual(core.invoke.mock.calls[0])
    expect(JSON.stringify(core.invoke.mock.calls)).not.toContain('passphrase":"')
  })

  it('cancelling the prompt gives `cancelled` and does not run the command again', async () => {
    setUnlockHandler(vi.fn().mockResolvedValue(false))
    core.invoke.mockRejectedValue(needs)
    await expect(api.repoStatus('/repos/a')).rejects.toMatchObject({ code: 'cancelled' })
    expect(core.invoke).toHaveBeenCalledTimes(1)
  })

  it('asks only once per failure: a second needs_input after unlocking is returned as it is', async () => {
    const handler = vi.fn().mockResolvedValue(true)
    setUnlockHandler(handler)
    core.invoke.mockRejectedValue(needs)
    await expect(api.repoStatus('/repos/a')).rejects.toMatchObject({ code: 'needs_input' })
    expect(handler).toHaveBeenCalledTimes(1)
    expect(core.invoke).toHaveBeenCalledTimes(2)
  })

  it('other errors, commands without a repo, and no handler pass straight through', async () => {
    const handler = vi.fn().mockResolvedValue(true)
    setUnlockHandler(handler)
    core.invoke.mockRejectedValueOnce({ code: 'error', message: 'x' })
    await expect(api.repoStatus('/repos/a')).rejects.toMatchObject({ code: 'error' })
    core.invoke.mockRejectedValueOnce(needs)
    await expect(api.authRun(null, 'status', null)).rejects.toMatchObject({ code: 'needs_input' })
    expect(handler).not.toHaveBeenCalled()
    setUnlockHandler(null)
    core.invoke.mockRejectedValueOnce(needs)
    await expect(api.repoStatus('/repos/a')).rejects.toMatchObject({ code: 'needs_input' })
  })

  it('commands that carry their own secrets never trigger the unlock prompt', async () => {
    const handler = vi.fn().mockResolvedValue(true)
    setUnlockHandler(handler)
    core.invoke.mockRejectedValue(needs)
    await expect(api.keyInit('/repos/a', 'pw')).rejects.toMatchObject({ code: 'needs_input' })
    await expect(api.keyRecover('/repos/a', 'CODE', null)).rejects.toMatchObject({ code: 'needs_input' })
    await expect(api.authLogin('/repos/a', null, 'm', 'pw')).rejects.toMatchObject({ code: 'needs_input' })
    expect(handler).not.toHaveBeenCalled()
  })
})

describe('a repository that still needs the passphrase is not "unlocked"', () => {
  it('needs_input right after an unlock moves the repo from unlocked to locked', async () => {
    const u = useUnlockStore()
    u.unlocked.add('/repos/a')
    setUnlockHandler(async () => true)
    setStillLockedHandler((r) => u.markLocked(r))
    core.invoke.mockRejectedValue(needs)
    await expect(api.repoStatus('/repos/a')).rejects.toMatchObject({ code: 'needs_input' })
    expect(u.unlocked.has('/repos/a')).toBe(false)
    expect(u.locked.has('/repos/a')).toBe(true)
  })

  it('asking marks the repo locked until the passphrase is accepted; cancelling leaves it locked', async () => {
    const u = useUnlockStore()
    u.unlocked.add('/repos/a')
    core.invoke.mockResolvedValue(undefined)
    const job = u.unlockRepo('/repos/a')
    expect(u.unlocked.has('/repos/a')).toBe(false)
    expect(u.locked.has('/repos/a')).toBe(true)
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer(null)
    await job
    expect(u.locked.has('/repos/a')).toBe(true)
    const again = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('pw')
    await again
    expect(u.unlocked.has('/repos/a')).toBe(true)
    expect(u.locked.has('/repos/a')).toBe(false)
  })
})

describe('unlockRepo (shared prompt)', () => {
  it('commands failing together share one dialog and one passphrase check', async () => {
    const u = useUnlockStore()
    core.invoke.mockResolvedValue(undefined)
    const a = u.unlockRepo('/repos/a')
    const b = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('pw')
    await expect(Promise.all([a, b])).resolves.toEqual([true, true])
    expect(core.invoke.mock.calls).toEqual([['unlock_repo', { repo: '/repos/a', passphrase: 'pw' }]])
    expect(JSON.stringify(u.$state)).not.toContain('pw')
  })

  it('a rejected passphrase asks again; cancelling ends it; a new prompt can follow', async () => {
    const u = useUnlockStore()
    core.invoke.mockRejectedValueOnce({ code: 'wrong_passphrase', message: 'no' }).mockResolvedValueOnce(undefined)
    const job = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('bad')
    await vi.waitFor(() => expect(u.retry).toBe(true))
    u.answer('good')
    await expect(job).resolves.toBe(true)
    const next = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer(null)
    await expect(next).resolves.toBe(false)
  })

  it('remembers which repositories are unlocked and forgets one on lock', async () => {
    const u = useUnlockStore()
    core.invoke.mockResolvedValue(undefined)
    const job = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('pw')
    await job
    expect(u.unlocked.has('/repos/a')).toBe(true)
    u.lock('/repos/a')
    expect(u.unlocked.has('/repos/a')).toBe(false)
    await vi.waitFor(() => expect(core.invoke).toHaveBeenCalledWith('lock_repo', { repo: '/repos/a' }))
  })

  it('a failure that is not a rejected passphrase is the answer, not a reason to ask again', async () => {
    const u = useUnlockStore()
    core.invoke.mockRejectedValue({ code: 'error', message: 'not a repository' })
    const job = u.unlockRepo('/repos/a')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('pw')
    await expect(job).rejects.toMatchObject({ message: 'not a repository' })
    expect(u.open).toBe(false)
    expect(u.unlocked.has('/repos/a')).toBe(false)
    expect(core.invoke).toHaveBeenCalledTimes(1)
  })
})

describe('concurrent prompts', () => {
  it('two commands that need a passphrase at once are each asked, one after the other', async () => {
    const u = useUnlockStore()
    const a = u.withUnlock(vi.fn().mockRejectedValueOnce(needs).mockResolvedValueOnce('A'))
    const b = u.withUnlock(vi.fn().mockRejectedValueOnce(needs).mockResolvedValueOnce('B'))
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('p1')
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('p2')
    expect(await Promise.all([a, b])).toEqual(['A', 'B'])
    expect(u.open).toBe(false)
  }, 2000)
})
