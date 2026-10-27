import { describe, expect, it } from 'vitest'
import type { LogCommitReport, LogRefReport, FetchReport, PullReport, PushReport } from '@/api'
import { ancestorsOf, decorate, graphWidth, headOid, matchesQuery } from '@/lib/history'
import { flattenInfo, hasMediaInfo, humanizeKey, joinPath, previewKind, refNameError } from '@/lib/inspect'
import { summarizeTransfer } from '@/lib/sync'
import { filterLocks } from '@/stores/locks'

const commit = (oid: string, parents: string[], summary = 's', name = 'Mira'): LogCommitReport => ({
  oid,
  parents,
  author: { name, email: `${name.toLowerCase()}@x.example` },
  time: 0,
  summary,
  message: summary,
})

describe('history helpers', () => {
  const refs: LogRefReport[] = [
    { name: 'HEAD', oid: 'c', kind: 'head' },
    { name: 'origin/main', oid: 'c', kind: 'remote' },
    { name: 'v1', oid: 'c', kind: 'tag' },
    { name: 'main', oid: 'c', kind: 'branch' },
    { name: 'feat', oid: 'b', kind: 'branch' },
  ]

  it('decorates commits with branches first, then tags, then remotes; HEAD is not a pill', () => {
    const d = decorate(refs)
    expect(d.get('c')!.map((p) => p.name)).toEqual(['main', 'v1', 'origin/main'])
    expect(d.get('b')).toEqual([{ name: 'feat', kind: 'branch' }])
    expect(headOid(refs)).toBe('c')
    expect(headOid([])).toBeNull()
  })

  it('finds the loaded ancestors of a tip in one pass (merges include both sides)', () => {
    // c merges b and a; b -> root; a -> root; x is on another branch
    const list = [commit('c', ['b', 'a']), commit('x', ['root']), commit('b', ['root']), commit('a', ['root']), commit('root', [])]
    expect([...ancestorsOf(list, 'c')].sort()).toEqual(['a', 'b', 'c', 'root'])
    expect(ancestorsOf(list, null).size).toBe(0)
  })

  it('a parent beyond the loaded pages is still listed, so the filter never loses the boundary', () => {
    expect(ancestorsOf([commit('c', ['gone'])], 'c').has('gone')).toBe(true)
  })

  it('matches search over summary, author, email and id prefix, case-insensitively', () => {
    const c = commit('abc123', [], 'Grade pass on Shot_010', 'Mira')
    for (const q of ['grade', 'SHOT_010', 'mira', 'mira@x', 'abc1', '  ']) expect(matchesQuery(c, q), q).toBe(true)
    for (const q of ['nothing', 'zzz']) expect(matchesQuery(c, q), q).toBe(false)
  })

  it('graph column is padding plus lane gaps', () => {
    expect(graphWidth(1)).toBe(16)
    expect(graphWidth(3)).toBe(16 + 28)
    expect(graphWidth(0)).toBe(16)
  })
})

describe('inspector helpers', () => {
  it('previews only what a WebView plays natively', () => {
    expect(previewKind('a/b/hero.PNG')).toBe('image')
    expect(previewKind('shot.mp4')).toBe('video')
    expect(previewKind('vo.wav')).toBe('audio')
    for (const p of ['hero.psd', 'sky.exr', 'rig.fbx', 'plate.tif', 'noext']) expect(previewKind(p), p).toBe('none')
  })

  it('knows which types media info can parse', () => {
    expect(hasMediaInfo('a.PSD')).toBe(true)
    expect(hasMediaInfo('a.exr')).toBe(false)
  })

  it('joins repo paths with the repo separator', () => {
    expect(joinPath('D:\\repos\\hero', 'art/hero.psd')).toBe('D:\\repos\\hero\\art\\hero.psd')
    expect(joinPath('D:\\repos\\hero\\', 'a.png')).toBe('D:\\repos\\hero\\a.png')
    expect(joinPath('/repos/hero', 'art/hero.psd')).toBe('/repos/hero/art/hero.psd')
  })

  it('turns a parser struct into rows: scalars, pairs as sizes, nested objects prefixed', () => {
    const rows = flattenInfo({
      width: 1920,
      color_mode: 'RGB',
      has_alpha: true,
      dimensions: [1920, 1080],
      file_size: 2048,
      nothing: null,
      codec: { name: 'h264', profile: 'High' },
      layers: [{ deep: 1 }],
    })
    expect(Object.fromEntries(rows.map((r) => [r.label, r.value]))).toEqual({
      Width: '1920',
      'Color mode': 'RGB',
      'Has alpha': 'Yes',
      Dimensions: '1920 × 1080',
      'File size': '2.0 KB',
      'Codec name': 'h264',
      'Codec profile': 'High',
    })
  })

  it('caps rows and rounds floats', () => {
    const many = Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`k${i}`, i]))
    expect(flattenInfo(many)).toHaveLength(24)
    expect(flattenInfo({ fps: 23.97602 })[0]!.value).toBe('23.98')
    expect(humanizeKey('color_mode')).toBe('Color mode')
  })

  it('validates branch and remote names like the desktop crate does', () => {
    for (const ok of ['main', 'feature/grade', 'v1.0-rc', 'a_b']) expect(refNameError(ok), ok).toBeNull()
    for (const bad of ['', ' ', '-x', 'a b', 'a..b', 'a~1', 'a^', 'a:b', 'x/', '/x', 'a//b', 'x.lock', 'x.', 'a@{1}', 'a\\b', 'a?', 'a*', 'a[']) {
      expect(refNameError(bad), JSON.stringify(bad)).not.toBeNull()
    }
  })
})

describe('transfer summaries and lock filters', () => {
  const fetchR = { branches_updated: 1, tags_updated: 2, bytes_downloaded: 3 * 1024 * 1024 } as FetchReport
  const pullR = (result: string) => ({ result, objects_received: 5, bytes_downloaded: 2048 }) as PullReport
  const pushR = (result: string) => ({ result, objects_sent: 14, bytes_uploaded: 2048 }) as PushReport

  it('says what happened in plain words', () => {
    expect(summarizeTransfer('fetch', fetchR)).toBe('Fetched 3 updates, 3.0 MB')
    expect(summarizeTransfer('fetch', { ...fetchR, branches_updated: 0, tags_updated: 0 })).toBe('Already up to date')
    expect(summarizeTransfer('pull', pullR('fast_forward'))).toBe('Got latest: 5 objects, 2.0 KB')
    expect(summarizeTransfer('pull', pullR('up_to_date'))).toBe('Already up to date')
    expect(summarizeTransfer('push', pushR('pushed'))).toBe('Sent 14 objects, 2.0 KB')
    expect(summarizeTransfer('push', pushR('up_to_date'))).toBe('Nothing new to send')
  })

  it('filters locks by owner; unknown identity means Mine is empty and Others is everything', () => {
    const locks = [
      { lock_id: '1', path: 'a', owner: 'mira', created_at: 0 },
      { lock_id: '2', path: 'b', owner: 'dev', created_at: 0 },
    ]
    expect(filterLocks(locks, 'all', null)).toHaveLength(2)
    expect(filterLocks(locks, 'mine', 'mira').map((l) => l.path)).toEqual(['a'])
    expect(filterLocks(locks, 'others', 'mira').map((l) => l.path)).toEqual(['b'])
    expect(filterLocks(locks, 'mine', null)).toHaveLength(0)
    expect(filterLocks(locks, 'others', null)).toHaveLength(2)
  })
})
