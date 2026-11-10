import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { enableAutoUnmount, mount } from '@vue/test-utils'

const api = vi.hoisted(() => ({ extract: vi.fn(), mediaInfo: vi.fn(), log: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { fileKey, useInspectorStore } from '@/stores/inspector'
import { useRepoStore } from '@/stores/repo'
import { useOpsStore } from '@/stores/ops'
import ActivityPill from '@/shell/ActivityPill.vue'
import Tooltip from '@/components/Tooltip.vue'

enableAutoUnmount(afterEach)

let inspector: ReturnType<typeof useInspectorStore>
let repo: ReturnType<typeof useRepoStore>
const version = { path: 'a.png', size: 5, rev: 'abc1' }

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  repo = useRepoStore()
  repo.path = '/r'
  inspector = useInspectorStore()
})

describe('inspector store, failure and refresh paths', () => {
  it('does nothing without an open repository', () => {
    repo.path = null
    inspector.ensureExtraction(version)
    inspector.ensureInfo(version)
    inspector.ensureHistory(version)
    expect(api.extract).not.toHaveBeenCalled()
    expect(inspector.extractions.size + inspector.infos.size + inspector.histories.size).toBe(0)
  })

  it('shows a failed extraction as an error and retries it from scratch', async () => {
    api.extract.mockRejectedValueOnce({ code: 'io', message: 'disk full' })
    inspector.ensureExtraction(version)
    await vi.waitFor(() => expect(inspector.extractions.get(fileKey(version))?.status).toBe('error'))
    expect(inspector.extractions.get(fileKey(version))?.error?.message).toBe('disk full')

    api.extract.mockResolvedValueOnce({ path: '/tmp/ok.png', size: 5 })
    inspector.retryExtraction(version)
    await vi.waitFor(() => expect(inspector.extractions.get(fileKey(version))?.status).toBe('ready'))
    expect(api.extract).toHaveBeenCalledTimes(2)
  })

  it('a failed extraction also fails the media info that waited for it', async () => {
    api.extract.mockRejectedValue({ code: 'io', message: 'gone' })
    inspector.ensureInfo(version)
    await vi.waitFor(() => expect(inspector.infos.get(fileKey(version))?.status).toBe('error'))
    expect(api.mediaInfo).not.toHaveBeenCalled()
  })

  it('keeps the progress length from the file size when the engine reports none', async () => {
    let report: (p: { pos: number; len: number | null }) => void = () => {}
    api.extract.mockImplementation((_r, _v, _p, _s, cb) => {
      report = cb
      return new Promise(() => {})
    })
    inspector.ensureExtraction(version)
    report({ pos: 2, len: null })
    expect(inspector.extractions.get(fileKey(version))).toMatchObject({ pos: 2, len: 5 })
  })

  it('loads a file history once, and records a failure', async () => {
    api.log.mockResolvedValueOnce({ commits: [{ oid: 'o1' }] })
    inspector.ensureHistory(version)
    inspector.ensureHistory(version)
    await vi.waitFor(() => expect(inspector.histories.get('a.png')?.status).toBe('ready'))
    expect(api.log).toHaveBeenCalledTimes(1)
    expect(api.log).toHaveBeenCalledWith('/r', 0, 10, false, 'a.png')

    api.log.mockRejectedValueOnce({ code: 'x', message: 'no log' })
    inspector.ensureHistory({ ...version, path: 'b.png' })
    await vi.waitFor(() => expect(inspector.histories.get('b.png')?.status).toBe('error'))
  })

  it('drops everything when another repository is opened', async () => {
    inspector.ensureExtraction({ path: 'w.png', size: 1, rev: null })
    api.log.mockResolvedValue({ commits: [] })
    inspector.ensureHistory(version)
    repo.path = '/other'
    await nextTick()
    expect(inspector.extractions.size).toBe(0)
    expect(inspector.histories.size).toBe(0)
  })
})

describe('activity pill', () => {
  it('is absent when nothing runs and appears while a push is running', async () => {
    const ops = useOpsStore()
    const w = mount(ActivityPill)
    expect(w.find('button').exists()).toBe(false)
    const past = Date.now() - 5000 // past the anti-flicker delay
    const id = ops.begin({ kind: 'push', label: 'Push to origin' }, past)
    ops.now = Date.now()
    await nextTick()
    expect(w.find('button').exists()).toBe(true)
    expect(w.text()).toContain('Push to origin')
    ops.fail(id, { code: 'x', message: 'offline' })
    await nextTick()
    expect(w.text()).toContain('Push to origin')
    expect(w.text()).toContain('failed')
    ops.reset()
  })
})

describe('tooltip', () => {
  it('renders the trigger it wraps', () => {
    const w = mount(Tooltip, { props: { text: 'Hint' }, slots: { default: '<button>Go</button>' } })
    expect(w.find('button').text()).toBe('Go')
  })
})
