import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  branchList: vi.fn(),
  log: vi.fn(),
  authRun: vi.fn(),
  keyStatus: vi.fn(),
  keyInit: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { useRepoStore } from '@/stores/repo'
import { useSecurityStore } from '@/stores/security'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' }, summary: {} })
  api.allowRepo.mockResolvedValue(undefined)
  api.branchList.mockResolvedValue({ current: 'main', branches: [] })
  api.log.mockResolvedValue({ commits: [], refs: [] })
  api.authRun.mockResolvedValue({ identity: null })
})

describe('BUG-047: recovery code with a passphrase', () => {
  it('is available as soon as key init returns, even while the follow-up status waits on the unlock prompt', async () => {
    useRepoStore().path = '/repos/hero'
    const sec = useSecurityStore()
    api.keyInit.mockResolvedValue({ action: 'init', enabled: true, source: 'interactive passphrase', recovery_code: 'RC-SECRET' })
    // After a passphrase init, key status fails needs_input and the api wrapper waits for the person to unlock.
    api.keyStatus.mockReturnValue(new Promise(() => {}))
    void sec.enableEncryption('pw')
    await vi.waitFor(() => expect(api.keyStatus).toHaveBeenCalled())
    expect(sec.recoveryCode).toBe('RC-SECRET')
  })
})

// T-08b: the real invoke wrapper, the real unlock store, both dialogs mounted; only the Tauri layer is faked.
describe('BUG-047 T-08b: through the real unlock prompt', () => {
  it('shows the recovery code after the person unlocks', async () => {
    vi.resetModules()
    vi.doUnmock('@/api')
    const core = { invoke: vi.fn() }
    vi.doMock('@tauri-apps/api/core', () => ({ invoke: core.invoke, convertFileSrc: (p: string) => p, Channel: class {} }))
    vi.doMock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
    const { mount, flushPromises } = await import('@vue/test-utils')
    const { defineComponent, h } = await import('vue')
    const { setUnlockHandler } = await import('@/api')
    const { useUnlockStore } = await import('@/stores/unlock')
    const { useRepoStore: repoStore } = await import('@/stores/repo')
    const { default: SecurityView } = await import('@/views/SecurityView.vue')
    const { default: UnlockDialog } = await import('@/shell/UnlockDialog.vue')
    const pinia = createPinia()
    setActivePinia(pinia)
    setUnlockHandler((repo) => useUnlockStore().unlockRepo(repo))
    let locked = true
    core.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'key_init') return { action: 'init', enabled: true, source: 'interactive passphrase', recovery_code: 'RC-SECRET' }
      if (cmd === 'unlock_repo') {
        locked = false
        return undefined
      }
      if (cmd === 'key_status') {
        if (locked) throw { code: 'needs_input', message: 'passphrase' }
        return { action: 'status', enabled: true, source: 'interactive passphrase', recovery_slot: true, recovery_code: null }
      }
      if (cmd === 'auth_run') return { identity: null }
      return { branch: { name: 'main' }, summary: {}, branches: [], current: 'main', commits: [], refs: [] }
    })
    repoStore().path = '/repos/hero'
    const w = mount(defineComponent({ render: () => h('div', [h(SecurityView), h(UnlockDialog)]) }), { global: { plugins: [pinia] }, attachTo: document.body })
    await flushPromises()
    const { useSecurityStore: secStore } = await import('@/stores/security')
    void secStore().enableEncryption('pw')
    await vi.waitFor(() => expect(useUnlockStore().open).toBe(true))
    useUnlockStore().answer('pw')
    await flushPromises()
    await vi.waitFor(() => expect(document.querySelector('[data-testid="recovery-code"]')?.textContent).toContain('RC-SECRET'))
    w.unmount()
  })
})
