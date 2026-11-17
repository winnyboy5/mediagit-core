import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { flushPromises, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter, RouterView } from 'vue-router'
import { defineComponent, h } from 'vue'

const api = vi.hoisted(() => ({ authRun: vi.fn(), keyStatus: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import SecurityView from '@/views/SecurityView.vue'
import { useSecurityStore } from '@/stores/security'
import { useUnlockStore } from '@/stores/unlock'

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.authRun.mockResolvedValue({ identity: null })
  api.keyStatus.mockResolvedValue({ action: 'status', enabled: false, source: null, recovery_slot: false, recovery_code: null })
})

describe('BUG-047: the held unlock prompt cannot be orphaned', () => {
  it('keeps the person on Security until the code dialog is closed', async () => {
    const pinia = createPinia()
    setActivePinia(pinia)
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/security', component: SecurityView },
        { path: '/other', component: { render: () => h('div') } },
      ],
    })
    await router.push('/security')
    mount(defineComponent({ render: () => h(RouterView) }), { global: { plugins: [pinia, router] } })
    await flushPromises()
    const sec = useSecurityStore()
    const unlock = useUnlockStore()
    sec.recoveryCode = 'RC-SECRET'
    void unlock.unlockRepo('/repos/hero')
    await router.push('/other')
    expect(router.currentRoute.value.path).toBe('/security')
    expect(unlock.open).toBe(false)
    sec.dismissRecovery()
    await vi.waitFor(() => expect(unlock.open).toBe(true))
    await router.push('/other')
    expect(router.currentRoute.value.path).toBe('/other')
  })
})
