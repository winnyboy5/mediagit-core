import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { defineComponent, nextTick } from 'vue'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'

const push = vi.hoisted(() => vi.fn())
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }))
const dialog = vi.hoisted(() => ({ open: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => dialog)

import { baseName, forget, load, save, timeAgo } from '@/lib/persist'
import { useFocusRefresh } from '@/lib/focusRefresh'
import { NARROW_PX, useShellStore } from '@/stores/shell'
import { useRecentStore } from '@/stores/recent'
import { useRepoStore } from '@/stores/repo'
import { useUnlockStore } from '@/stores/unlock'
import { pickFolder, useOpenRepo } from '@/shell/useOpenRepo'
import UnlockDialog from '@/shell/UnlockDialog.vue'
import Wordmark from '@/components/Wordmark.vue'

enableAutoUnmount(afterEach)

beforeEach(() => {
  localStorage.clear()
  setActivePinia(createPinia())
  push.mockReset()
  dialog.open.mockReset()
})

describe('persist', () => {
  it('round-trips JSON and returns the fallback for missing or corrupt values', () => {
    save('k', { a: 1 })
    expect(load('k', null)).toEqual({ a: 1 })
    expect(load('missing', 'fb')).toBe('fb')
    localStorage.setItem('bad', '{not json')
    expect(load('bad', 'fb')).toBe('fb')
    forget('k')
    expect(load('k', 0)).toBe(0)
  })

  it('keeps working when storage throws', () => {
    const boom = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    const rm = vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    expect(() => save('k', 1)).not.toThrow()
    expect(() => forget('k')).not.toThrow()
    boom.mockRestore()
    rm.mockRestore()
  })

  it('names a folder by its last segment with either slash style', () => {
    expect(baseName('C:\\work\\film\\')).toBe('film')
    expect(baseName('/a/b/c')).toBe('c')
  })

  it('writes ages the way people say them', () => {
    const now = 10_000_000_000
    const ago = (s: number) => timeAgo(now - s * 1000, now)
    expect(ago(5)).toBe('just now')
    expect(ago(120)).toBe('2m ago')
    expect(ago(3 * 3600)).toBe('3h ago')
    expect(ago(2 * 86400)).toBe('2d ago')
    expect(ago(14 * 86400)).toBe('2w ago')
    expect(ago(90 * 86400)).toBe('3mo ago')
    expect(ago(400 * 86400)).toBe('13mo ago')
  })
})

describe('focus refresh', () => {
  it('re-reads when the window regains focus, and stops after unmount', () => {
    const fn = vi.fn()
    const w = mount(defineComponent({ setup: () => (useFocusRefresh(fn), () => null) }))
    window.dispatchEvent(new Event('focus'))
    expect(fn).toHaveBeenCalledTimes(1)
    w.unmount()
    window.dispatchEvent(new Event('focus'))
    expect(fn).toHaveBeenCalledTimes(1)
  })
})

describe('shell store', () => {
  it('toggles the inspector and remembers the choice', () => {
    const shell = useShellStore()
    shell.narrow = false
    expect(shell.inspectorVisible).toBe(true)
    shell.toggleInspector()
    expect(shell.inspectorVisible).toBe(false)
    expect(load('mg.inspector', true)).toBe(false)
  })

  it('on a narrow window the inspector is an overlay that is not remembered', () => {
    const shell = useShellStore()
    shell.narrow = true
    expect(shell.inspectorVisible).toBe(false)
    shell.toggleInspector()
    expect(shell.inspectorVisible).toBe(true)
    expect(shell.inspectorOpen).toBe(true)
    expect(localStorage.getItem('mg.inspector')).toBeNull()
  })

  it('follows the viewport media query until the scope is disposed', () => {
    let listener: () => void = () => {}
    const mq = { matches: true, addEventListener: (_: string, cb: () => void) => (listener = cb), removeEventListener: vi.fn() }
    const mm = vi.spyOn(window, 'matchMedia').mockReturnValue(mq as never)
    const shell = useShellStore()
    const w = mount(defineComponent({ setup: () => (shell.watchViewport(), () => null) }))
    expect(mm).toHaveBeenCalledWith(`(max-width: ${NARROW_PX - 1}px)`)
    expect(shell.narrow).toBe(true)
    mq.matches = false
    listener()
    expect(shell.narrow).toBe(false)
    w.unmount()
    expect(mq.removeEventListener).toHaveBeenCalled()
    mm.mockRestore()
  })
})

describe('recent store', () => {
  it('puts the newest first, de-duplicates by path and caps at eight', () => {
    const recent = useRecentStore()
    for (let i = 0; i < 10; i++) recent.add(`/r/${i}`)
    expect(recent.list).toHaveLength(8)
    expect(recent.list[0]!.path).toBe('/r/9')
    recent.add('/r/5', 'dev')
    expect(recent.list[0]).toMatchObject({ path: '/r/5', name: '5', branch: 'dev' })
    expect(recent.list.filter((r) => r.path === '/r/5')).toHaveLength(1)
  })

  it('removes an entry, persists, and reloads without malformed rows', () => {
    const recent = useRecentStore()
    recent.add('/a')
    recent.add('/b')
    recent.remove('/a')
    expect(recent.list.map((r) => r.path)).toEqual(['/b'])
    setActivePinia(createPinia())
    expect(useRecentStore().list.map((r) => r.path)).toEqual(['/b'])
    save('mg.recent', [null, { nope: 1 }, { path: '/ok', name: 'ok', openedAt: 1 }])
    setActivePinia(createPinia())
    expect(useRecentStore().list.map((r) => r.path)).toEqual(['/ok'])
  })
})

describe('opening a repository', () => {
  it('lands on the files view when the folder opens', async () => {
    const repo = useRepoStore()
    vi.spyOn(repo, 'open').mockResolvedValue(true)
    expect(await useOpenRepo().openPath('/r')).toBeNull()
    expect(push).toHaveBeenCalledWith('/files/all')
  })

  it('hands back the repo error and stays put when it fails', async () => {
    const repo = useRepoStore()
    vi.spyOn(repo, 'open').mockResolvedValue(false)
    repo.error = { code: 'not_a_repo', message: 'Not a repository' }
    expect(await useOpenRepo().openPath('/r')).toEqual({ code: 'not_a_repo', message: 'Not a repository' })
    expect(push).not.toHaveBeenCalled()
    repo.error = null
    expect((await useOpenRepo().openPath('/r'))?.code).toBe('unknown')
  })

  it('does nothing when the picker is cancelled, and reports a picker failure', async () => {
    dialog.open.mockResolvedValueOnce(null)
    expect(await useOpenRepo().pickAndOpen()).toBeNull()
    expect(await pickFolder('t')).toBeNull()
    dialog.open.mockRejectedValueOnce(new Error('no dialog'))
    expect(await useOpenRepo().pickAndOpen()).toEqual({ code: 'unknown', message: 'no dialog' })
    const repo = useRepoStore()
    vi.spyOn(repo, 'open').mockResolvedValue(true)
    dialog.open.mockResolvedValueOnce('/picked')
    expect(await useOpenRepo().pickAndOpen()).toBeNull()
    expect(repo.open).toHaveBeenCalledWith('/picked')
  })
})

describe('unlock dialog', () => {
  const field = () => document.body.querySelector<HTMLInputElement>('#unlock-form input')
  const unlockBtn = () => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Unlock')

  it('stays closed until a command asks for the passphrase', async () => {
    mount(UnlockDialog, { attachTo: document.body })
    await nextTick()
    expect(field()).toBeNull()
  })

  it('sends the typed passphrase once and clears the field', async () => {
    const unlock = useUnlockStore()
    mount(UnlockDialog, { attachTo: document.body })
    const answer = vi.spyOn(unlock, 'answer')
    unlock.open = true
    await flushPromises()
    expect(unlockBtn()?.disabled).toBe(true)
    const input = field()!
    input.value = 'secret'
    input.dispatchEvent(new Event('input'))
    await nextTick()
    document.body.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }))
    expect(answer).toHaveBeenCalledWith('secret')
    await nextTick()
    expect(unlock.open).toBe(false)
  })

  it('ignores an empty submit, and Cancel answers null', async () => {
    const unlock = useUnlockStore()
    mount(UnlockDialog, { attachTo: document.body })
    const answer = vi.spyOn(unlock, 'answer')
    unlock.open = true
    await flushPromises()
    document.body.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }))
    expect(answer).not.toHaveBeenCalled()
    ;[...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Cancel')!.click()
    expect(answer).toHaveBeenCalledWith(null)
  })

  it('says so when the previous passphrase was wrong', async () => {
    const unlock = useUnlockStore()
    mount(UnlockDialog, { attachTo: document.body })
    unlock.retry = true
    unlock.open = true
    await flushPromises()
    expect(document.body.textContent).toContain('That passphrase did not unlock the repository.')
  })
})

describe('wordmark', () => {
  it('is announced as MediaGit and sizes both logos', () => {
    const w = mount(Wordmark, { props: { height: 20 } })
    expect(w.attributes('aria-label')).toBe('MediaGit')
    expect(w.findAll('img').map((i) => i.attributes('height'))).toEqual(['20', '20'])
  })
})
