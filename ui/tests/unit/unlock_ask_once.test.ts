import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
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
import ErrorState from '@/components/ErrorState.vue'
import { useRepoStore } from '@/stores/repo'
import { useUnlockStore } from '@/stores/unlock'

const REPO = '/repos/a'
const needs = { code: 'needs_input', message: 'passphrase needed' }

/** Resolves 'quiet' if no prompt opens within a short time, else 'prompted'. */
async function quiet(u: ReturnType<typeof useUnlockStore>): Promise<'quiet' | 'prompted'> {
  for (let i = 0; i < 10; i++) {
    if (u.open) return 'prompted'
    await new Promise((r) => setTimeout(r, 10))
  }
  return 'quiet'
}

async function cancelOnce(u: ReturnType<typeof useUnlockStore>) {
  const job = u.unlockRepo(REPO)
  await vi.waitFor(() => expect(u.open).toBe(true))
  u.answer(null)
  expect(await job).toBe(false)
}

beforeEach(() => {
  setActivePinia(createPinia())
  core.invoke.mockReset()
  setStillLockedHandler(null)
  const u = useUnlockStore()
  setUnlockHandler((repo) => u.unlockForCommand(repo))
})

describe('the unlock prompt asks once', () => {
  it('after Cancel, commands do not prompt again and the repo stays locked', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    expect(await u.unlockForCommand(REPO)).toBe('declined')
    expect(await u.unlockForCommand(REPO)).toBe('declined')
    expect(await quiet(u)).toBe('quiet')
    expect(u.locked.has(REPO)).toBe(true)
    expect(u.unlocked.has(REPO)).toBe(false)
  })

  it('through the api layer a command fails as needs_input (not cancelled) after a Cancel, and ErrorState offers Unlock', async () => {
    const u = useUnlockStore()
    core.invoke.mockRejectedValue(needs)
    const first = api.repoStatus(REPO).catch((e) => e)
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer(null)
    expect(await first).toMatchObject({ code: 'cancelled' }) // the Cancel the person just clicked

    const again = await api.repoStatus(REPO).catch((e) => e)
    expect(again).toMatchObject({ code: 'needs_input' })
    expect(await quiet(u)).toBe('quiet')

    const w = mount(ErrorState, { props: { code: again.code, message: again.message } })
    expect(w.text()).toContain('Unlock')
  })

  it('an explicit unlock (the Sidebar and ErrorState buttons call unlockRepo) prompts again', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    await cancelOnce(u)
  })

  it('lock() clears the declined state', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    u.lock(REPO)
    const next = u.unlockForCommand(REPO)
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    await next
  })

  it('a successful unlock clears the declined state', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    core.invoke.mockResolvedValue(undefined)
    const job = u.unlockRepo(REPO)
    await vi.waitFor(() => expect(u.open).toBe(true))
    u.answer('pw')
    expect(await job).toBe(true)
    u.unlocked.delete(REPO)
    const next = u.unlockForCommand(REPO)
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    await next
  })

  it('opening the repository again resumes prompting (through repo.open)', async () => {
    const u = useUnlockStore()
    core.invoke.mockRejectedValue(needs)
    await cancelOnce(u)
    const opened = useRepoStore().open(REPO)
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    expect(await opened).toBe(false)
  })

  it('a cancel on one repository does not silence another', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    const other = u.unlockForCommand('/repos/b')
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    await other
  })
})
