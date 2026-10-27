import { describe, expect, it } from 'vitest'
import { Box, File, Film, Image, Layers, Sparkles, AudioLines, FileText } from 'lucide-vue-next'
import { mediaKind } from '@/components/media'
import { formatBytes, formatEta, formatRate } from '@/components/format'

describe('mediaKind', () => {
  it.each([
    ['a/b/plate.exr', 'vfx', Sparkles],
    ['x.PNG', 'image', Image],
    ['shot.mov', 'video', Film],
    ['take.mxf', 'video', Film],
    ['mix.wav', 'audio', AudioLines],
    ['rig.blend', 'model3d', Box],
    ['scene.usdz', 'model3d', Box],
    ['smoke.vdb', 'vfx', Sparkles],
    ['brief.pdf', 'document', FileText],
    ['art.psd', 'document', Layers],
    ['comp.nk', 'vfx', Sparkles],
    ['rig.mb', 'model3d', Box],
    ['sim.hip', 'model3d', Box],
    ['model.glb', 'model3d', Box],
    ['paint.kra', 'document', Layers],
  ])('%s -> %s', (path, kind, icon) => {
    const m = mediaKind(path)
    expect(m.kind).toBe(kind)
    expect(m.icon).toBe(icon)
    expect(m.hueVar).toBe(`--media-${kind}`)
    expect(m.softVar).toBe(`--media-${kind}-soft`)
  })

  it('falls back to other for unknown or extensionless files', () => {
    expect(mediaKind('LICENSE').kind).toBe('other')
    expect(mediaKind('.gitignore').kind).toBe('other')
    expect(mediaKind('data.xyz').icon).toBe(File)
  })

  it('handles windows separators and dotted directories', () => {
    expect(mediaKind('C:\\shots.v2\\clip').kind).toBe('other')
    expect(mediaKind('C:\\shots.v2\\clip.mp4').kind).toBe('video')
  })
})

describe('format', () => {
  it('formats bytes', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(formatBytes(3.4e9)).toBe('3.2 GB')
  })
  it('shows -- for unknown rate and eta', () => {
    expect(formatRate(null)).toBe('--')
    expect(formatRate(undefined)).toBe('--')
    expect(formatEta(null)).toBe('--')
    expect(formatRate(1048576)).toBe('1.0 MB/s')
    expect(formatEta(75)).toBe('1m 15s')
    expect(formatEta(7500)).toBe('2h 5m')
  })
})
