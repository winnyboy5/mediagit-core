import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'

const api = vi.hoisted(() => ({
  repoStatus: vi.fn(),
  allowRepo: vi.fn(),
  remoteList: vi.fn(),
  remoteLsTree: vi.fn(),
  downloadPreview: vi.fn(),
  download: vi.fn(),
  cancelOp: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { breadcrumbs, downloadTarget, joinOut, sortEntries } from '@/lib/remoteBrowse'
import { DEFAULT_REF, useRemoteBrowserStore } from '@/stores/remoteBrowser'
import { useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

const E = (name: string, kind: 'file' | 'dir', path = name, size: number | null = 10) => ({ name, path, kind, size, oid: 'ab' })
const listing = (path: string, entries = [E('hero.png', 'file')]) => ({ remote: 'origin', ref: 'main', path, entries })

beforeEach(() => {
  setActivePinia(createPinia())
  vi.resetAllMocks()
  api.repoStatus.mockResolvedValue({ branch: { name: 'main' } })
  api.allowRepo.mockResolvedValue(undefined)
  api.cancelOp.mockResolvedValue(true)
  api.remoteList.mockResolvedValue({ remotes: [{ name: 'origin', url: 'https://h.example/r' }, { name: 'backup', url: 'https://b.example/r/' }] })
})

async function ready() {
  useRepoStore().path = '/repos/hero'
  await useSyncStore().refresh()
  return useRemoteBrowserStore()
}

describe('remote browse lib', () => {
  it('builds a breadcrumb per folder level', () => {
    expect(breadcrumbs('origin', '')).toEqual([{ label: 'origin', path: '' }])
    expect(breadcrumbs('origin', 'video/shots')).toEqual([
      { label: 'origin', path: '' },
      { label: 'video', path: 'video' },
      { label: 'shots', path: 'video/shots' },
    ])
  })

  it('addresses origin by relative path and other http remotes by URL', () => {
    expect(downloadTarget({ name: 'origin', url: 'https://h/r' }, 'a/b.png')).toBe('a/b.png')
    expect(downloadTarget({ name: 'backup', url: 'https://b/r/' }, 'a/b.png')).toBe('https://b/r/a/b.png')
    expect(downloadTarget({ name: 'local', url: '/srv/r' }, 'a/b.png')).toBe('a/b.png')
  })

  it('joins a folder and file name with the folder separator', () => {
    expect(joinOut('C:\\Users\\M\\Desktop', 'a.png')).toBe('C:\\Users\\M\\Desktop\\a.png')
    expect(joinOut('/home/m/', 'a.png')).toBe('/home/m/a.png')
  })

  it('sorts folders before files, each by name', () => {
    expect(sortEntries([E('b.png', 'file'), E('z', 'dir'), E('a', 'dir'), E('a.png', 'file')]).map((e) => e.name)).toEqual(['a', 'z', 'a.png', 'b.png'])
  })
})

describe('remote browser store', () => {
  it('lists one level at a time with the default ref, then a chosen one', async () => {
    const rb = await ready()
    api.remoteLsTree.mockResolvedValue(listing(''))
    await rb.load()
    expect(api.remoteLsTree).toHaveBeenLastCalledWith('/repos/hero', 'origin', null, null)
    api.remoteLsTree.mockResolvedValue(listing('video'))
    await rb.open('video')
    expect(api.remoteLsTree).toHaveBeenLastCalledWith('/repos/hero', 'origin', 'video', null)
    expect(rb.path).toBe('video')
    rb.ref = 'v0.3'
    await rb.load()
    expect(api.remoteLsTree).toHaveBeenLastCalledWith('/repos/hero', 'origin', null, 'v0.3')
  })

  it('keeps the last listing when a later one fails, with the error', async () => {
    const rb = await ready()
    api.remoteLsTree.mockResolvedValueOnce(listing(''))
    await rb.load()
    api.remoteLsTree.mockRejectedValueOnce({ code: 'error', message: 'timeout' })
    await rb.open('video')
    expect(rb.error?.message).toBe('timeout')
    expect(rb.listing?.path).toBe('')
  })

  it('previews images, audio and video only; others are not fetched', async () => {
    const rb = await ready()
    api.downloadPreview.mockResolvedValue({ path: '/p/hero.png', size: 10 })
    rb.select(E('notes.txt', 'file'))
    expect(api.downloadPreview).not.toHaveBeenCalled()
    rb.select(E('folder', 'dir'))
    expect(api.downloadPreview).not.toHaveBeenCalled()
    rb.select(E('hero.png', 'file'))
    expect(rb.preview?.status).toBe('loading')
    await vi.waitFor(() => expect(rb.preview?.status).toBe('ready'))
    expect(rb.preview?.path).toBe('/p/hero.png')
    expect(api.downloadPreview).toHaveBeenCalledWith('/repos/hero', 'hero.png', null, expect.stringMatching(/^preview-/), expect.any(Function))
  })

  it('cancels the old preview when another file is picked and ignores its late result', async () => {
    const rb = await ready()
    let late!: (v: unknown) => void
    api.downloadPreview.mockImplementationOnce(() => new Promise((r) => (late = r)))
    const first = E('a.png', 'file')
    rb.select(first)
    const id = api.downloadPreview.mock.calls[0]![3]
    rb.select(E('b.txt', 'file'))
    expect(api.cancelOp).toHaveBeenCalledWith(id)
    late({ path: '/p/a.png', size: 1 })
    await nextTick()
    expect(rb.preview).toBeNull()
  })

  it('streams preview bytes into the bar', async () => {
    const rb = await ready()
    api.downloadPreview.mockImplementation((_r, _p, _g, _i, cb) => {
      cb({ id: 0, msg: 'x', pos: 4, len: 8, unit: 'bytes', done: false })
      return new Promise(() => {})
    })
    rb.select(E('a.png', 'file', 'a.png', 100))
    expect(rb.preview).toMatchObject({ status: 'loading', pos: 4, len: 8 })
  })

  it('reports a preview failure but not a cancellation', async () => {
    const rb = await ready()
    api.downloadPreview.mockRejectedValueOnce({ code: 'error', message: 'gone' })
    rb.select(E('a.png', 'file'))
    await vi.waitFor(() => expect(rb.preview?.status).toBe('error'))
    expect(rb.preview?.error?.message).toBe('gone')
  })

  it('downloads the selected file into the chosen folder as a cancellable op', async () => {
    const rb = await ready()
    api.downloadPreview.mockReturnValue(new Promise(() => {}))
    rb.select(E('shot.mp4', 'file', 'video/shot.mp4', 100))
    api.download.mockResolvedValue({ path: 'video/shot.mp4', output: '/out/shot.mp4', bytes: 100, source: 'remote' })
    expect(await rb.download('/out')).toBe(true)
    expect(api.download).toHaveBeenCalledWith('/repos/hero', 'video/shot.mp4', null, '/out/shot.mp4', expect.stringMatching(/^op-/), expect.any(Function))
    const op = useOpsStore().finished[0]!
    expect(op.kind).toBe('download')
    expect(op.summary).toContain('/out/shot.mp4')
  })

  it('addresses a non-origin remote by URL and keeps a failure; a cancel is silent', async () => {
    const rb = await ready()
    rb.remoteName = 'backup'
    rb.ref = 'v1'
    rb.select(E('a.psd', 'file', 'art/a.psd'))
    api.download.mockRejectedValueOnce({ code: 'error', message: 'disk full' })
    expect(await rb.download('/out')).toBe(false)
    expect(api.download.mock.calls[0]!.slice(1, 5)).toEqual(['https://b.example/r/art/a.psd', 'v1', '/out/a.psd', expect.any(String)])
    expect(rb.downloadError?.message).toBe('disk full')
    api.download.mockRejectedValueOnce({ code: 'cancelled', message: 'Cancelled' })
    expect(await rb.download('/out')).toBe(false)
    expect(rb.downloadError).toBeNull()
  })

  it('resets when the repository changes', async () => {
    const rb = await ready()
    api.remoteLsTree.mockResolvedValue(listing(''))
    await rb.load()
    rb.ref = 'v1'
    useRepoStore().path = '/repos/other'
    await nextTick()
    expect(rb.listing).toBeNull()
    expect(rb.ref).toBe(DEFAULT_REF)
  })
})
