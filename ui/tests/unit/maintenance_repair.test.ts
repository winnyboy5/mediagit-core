import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({ fsck: vi.fn(), purgeQuarantine: vi.fn() }))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { repairVerdict, useMaintenanceStore } from '@/stores/maintenance'
import { useRepoStore } from '@/stores/repo'

const report = (over: Record<string, unknown> = {}) => ({
  healthy: true,
  objects_checked: 10,
  refs_checked: 2,
  corrupted_objects: 0,
  broken_refs: 0,
  missing_objects: 0,
  dangling_objects: 0,
  repaired: 0,
  restored: 0,
  quarantined: 0,
  refs_removed: 0,
  unrestorable_files: [],
  repair_errors: [],
  quarantine_count: 0,
  quarantine_bytes: 0,
  issues: [],
  ...over,
})

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  useRepoStore().path = '/repos/hero'
})

describe('repair outcome', () => {
  it('is clean only when the re-check is healthy and nothing is lost or failed', () => {
    expect(repairVerdict(report({ restored: 2 }) as never)).toBe('clean')
    expect(repairVerdict(report({ unrestorable_files: ['a.psd'] }) as never)).toBe('lost')
    expect(repairVerdict(report({ repair_errors: ['could not write'] }) as never)).toBe('errors')
    expect(repairVerdict(report({ healthy: false }) as never)).toBe('problems')
    // Lost files win over everything: never a success state.
    expect(repairVerdict(report({ unrestorable_files: ['a.psd'], repair_errors: ['x'], healthy: false }) as never)).toBe('lost')
  })

  it('a repair run is remembered with its figures; a healthy re-check with lost files is not a success', async () => {
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue(report({ quarantined: 1, unrestorable_files: ['art/hero.psd'], quarantine_count: 1, quarantine_bytes: 2048 }))
    await m.runFsck({ repair: true, confirmed: true })
    expect(m.fsck?.repair).toBe('done')
    expect(m.fsckGood).toBe(false)
    expect(m.quarantine).toEqual({ count: 1, bytes: 2048 })
  })

  it('a plain healthy check is good; a clean repair is good', async () => {
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue(report())
    await m.runFsck({})
    expect(m.fsckGood).toBe(true)
    api.fsck.mockResolvedValue(report({ restored: 3 }))
    await m.runFsck({ repair: true, confirmed: true })
    expect(m.fsckGood).toBe(true)
  })
})

describe('quarantine purge', () => {
  it('is hidden when the quarantine holds nothing, and never calls the CLI', async () => {
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue(report())
    await m.runFsck({})
    expect(m.quarantine).toBeNull()
    expect(await m.purgeQuarantine(true)).toBe(false)
    expect(api.purgeQuarantine).not.toHaveBeenCalled()
  })

  it('needs the confirmation; without it nothing is sent', async () => {
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue(report({ quarantine_count: 3, quarantine_bytes: 900 }))
    await m.runFsck({})
    expect(await m.purgeQuarantine(false)).toBe(false)
    expect(api.purgeQuarantine).not.toHaveBeenCalled()
  })

  it('confirmed: deletes, then the quarantine line goes and the result is kept', async () => {
    const m = useMaintenanceStore()
    api.fsck.mockResolvedValue(report({ quarantine_count: 3, quarantine_bytes: 900 }))
    await m.runFsck({})
    api.purgeQuarantine.mockResolvedValue({ deleted_count: 3, deleted_bytes: 900, unrestorable_files: ['a.psd'] })
    expect(await m.purgeQuarantine(true)).toBe(true)
    expect(api.purgeQuarantine).toHaveBeenCalledWith('/repos/hero', undefined, expect.any(Function))
    expect(m.quarantine).toBeNull()
    expect(m.purged).toEqual({ deleted_count: 3, deleted_bytes: 900, unrestorable_files: ['a.psd'] })
    // A new check forgets the old purge result.
    api.fsck.mockResolvedValue(report())
    await m.runFsck({})
    expect(m.purged).toBeNull()
  })
})
