import { beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { routerKey } from 'vue-router'

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
import { useStorageStore } from '@/stores/storage'
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

describe('views that read while locked', () => {
  it('ErrorState offers Unlock for a locked repo even when the error is "cancelled"', async () => {
    const u = useUnlockStore()
    const repo = useRepoStore()
    repo.path = REPO
    const w = mount(ErrorState, { props: { code: 'cancelled', message: 'Cancelled' } })
    expect(w.text()).not.toContain('Unlock')
    u.locked.add(REPO)
    await w.vm.$nextTick()
    expect(w.text()).toContain('Unlock')
    expect(w.text()).not.toContain('Stopped')
  })

  it('withUnlock for a call nobody clicked respects a Cancel; a click still prompts', async () => {
    const u = useUnlockStore()
    await cancelOnce(u)
    const fn = vi.fn().mockRejectedValue(needs)
    await expect(u.withUnlock(fn, REPO)).rejects.toMatchObject({ code: 'needs_input' })
    expect(await quiet(u)).toBe('quiet')
    const clicked = u.withUnlock(fn).catch((e) => e)
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    expect(await clicked).toMatchObject({ code: 'cancelled' })
  })

  it('the storage view refresh on mount does not prompt after a Cancel; the button does', async () => {
    const u = useUnlockStore()
    useRepoStore().path = REPO
    core.invoke.mockRejectedValue(needs)
    await cancelOnce(u)
    const st = useStorageStore()
    await st.refresh()
    expect(st.error).toMatchObject({ code: 'needs_input' })
    expect(await quiet(u)).toBe('quiet')
    const clicked = st.refresh(true)
    expect(await quiet(u)).toBe('prompted')
    u.answer(null)
    await clicked
  })
})

describe('ErrorState scope', () => {
  it('a non-lock error on a locked repo keeps its own text and button', async () => {
    const u = useUnlockStore()
    useRepoStore().path = REPO
    u.locked.add(REPO)
    const w = mount(ErrorState, { props: { code: 'error', message: "Remote 'origin' not found in configuration" }, global: { provide: { [routerKey as symbol]: { push: () => {} } } } })
    expect(w.text()).not.toContain('This repository is locked')
    expect(w.text()).not.toContain('Unlock')
    expect(w.text()).toContain('Add remote')
  })
})
