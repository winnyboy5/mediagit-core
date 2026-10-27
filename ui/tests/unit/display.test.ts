import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import BranchPill from '@/components/BranchPill.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import FileIcon from '@/components/FileIcon.vue'
import FileRow from '@/components/FileRow.vue'
import HeadRing from '@/components/HeadRing.vue'
import Kbd from '@/components/Kbd.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import TagPill from '@/components/TagPill.vue'
import TransferStats from '@/components/TransferStats.vue'

describe('ProgressBar', () => {
  it('determinate exposes aria-valuenow/max', () => {
    const w = mount(ProgressBar, { props: { label: 'Upload', value: 25, max: 50 } })
    const el = w.get('[role=progressbar]')
    expect(el.attributes('aria-valuenow')).toBe('25')
    expect(el.attributes('aria-valuemax')).toBe('50')
    expect(el.attributes('aria-label')).toBe('Upload')
    expect(w.get('.fill').attributes('style')).toContain('width: 50%')
    expect(w.classes()).not.toContain('indeterminate')
  })
  it('indeterminate omits aria-valuenow and fakes no width', () => {
    const w = mount(ProgressBar, { props: { label: 'Chunking', value: null } })
    expect(w.attributes('aria-valuenow')).toBeUndefined()
    expect(w.classes()).toContain('indeterminate')
    expect(w.get('.fill').attributes('style')).toBeUndefined()
  })
  it('clamps and supports sm', () => {
    const w = mount(ProgressBar, { props: { label: 'x', value: 300, size: 'sm' } })
    expect(w.get('.fill').attributes('style')).toContain('width: 100%')
    expect(w.classes()).toContain('sm')
  })
})

describe('TransferStats', () => {
  it('shows -- for unknown rate and eta', () => {
    const t = mount(TransferStats, { props: { done: 1024, total: 2048 } }).text()
    expect(t).toContain('1.0 KB / 2.0 KB')
    expect(t).toContain('--')
    expect(t).toContain('ETA --')
  })
  it('shows measured values', () => {
    const t = mount(TransferStats, { props: { done: 1e6, total: 2e6, rate: 1048576, eta: 30 } }).text()
    expect(t).toContain('1.0 MB/s')
    expect(t).toContain('ETA 30s')
  })
})

describe('StatusBadge', () => {
  it.each([
    ['added', 'A', 'Added'],
    ['modified', 'M', 'Modified'],
    ['deleted', 'D', 'Deleted'],
    ['locked', 'L', 'Locked'],
    ['untracked', 'U', 'Untracked'],
    ['conflict', '!', 'Conflict'],
  ] as const)('%s has letter, icon and accessible name', (status, letter, word) => {
    const w = mount(StatusBadge, { props: { status } })
    expect(w.text()).toBe(letter)
    expect(w.find('svg').exists()).toBe(true)
    expect(w.attributes('aria-label')).toBe(word)
  })
})

describe('FileIcon / FileRow', () => {
  it('FileIcon tints by media kind', () => {
    const w = mount(FileIcon, { props: { path: 'a.exr' } })
    expect(w.attributes('data-kind')).toBe('vfx')
    expect(w.attributes('style')).toContain('--media-vfx')
  })
  it('FileRow splits dir/name, formats size, shows status and progress', () => {
    const w = mount(FileRow, { props: { path: 'shots/a.mov', size: 1536, status: 'added', progress: 40 } })
    expect(w.get('.dir').text()).toBe('shots/')
    expect(w.get('.name').text()).toBe('a.mov')
    expect(w.get('.size').text()).toBe('1.5 KB')
    expect(w.findComponent(StatusBadge).exists()).toBe(true)
    expect(w.get('[role=progressbar]').attributes('aria-valuenow')).toBe('40')
  })
  it('FileRow without progress renders no bar; null renders indeterminate', () => {
    expect(mount(FileRow, { props: { path: 'a.png' } }).find('[role=progressbar]').exists()).toBe(false)
    const w = mount(FileRow, { props: { path: 'a.png', progress: null } })
    expect(w.get('[role=progressbar]').attributes('aria-valuenow')).toBeUndefined()
  })
})

describe('pills', () => {
  it('BranchPill marks the current branch for screen readers', () => {
    const w = mount(BranchPill, { props: { name: 'main', current: true, lane: 2 } })
    expect(w.text()).toContain('main')
    expect(w.text()).toContain('current branch')
    expect(w.attributes('style')).toContain('--lane-2')
  })
  it('TagPill and HeadRing', () => {
    expect(mount(TagPill, { props: { name: 'v1' } }).text()).toBe('v1')
    expect(mount(HeadRing).attributes('aria-label')).toBe('HEAD (current)')
  })
})

describe('states', () => {
  it('EmptyState renders title, description and action slot', () => {
    const w = mount(EmptyState, { props: { title: 'No changes', description: 'All saved' }, slots: { default: '<button>Go</button>' } })
    expect(w.get('h3').text()).toBe('No changes')
    expect(w.text()).toContain('All saved')
    expect(w.find('button').exists()).toBe(true)
  })
  it('ErrorState shows code + message and emits retry', async () => {
    const w = mount(ErrorState, { props: { code: 'E_X', message: 'Nope' } })
    expect(w.attributes('role')).toBe('alert')
    expect(w.text()).toContain('E_X')
    expect(w.text()).toContain('Nope')
    await w.get('button').trigger('click')
    expect(w.emitted('retry')).toHaveLength(1)
  })
  it('SkeletonRows renders count rows with a status label', () => {
    const w = mount(SkeletonRows, { props: { count: 3 } })
    expect(w.findAll('.row')).toHaveLength(3)
    expect(w.attributes('aria-label')).toBe('Loading')
  })
  it('Kbd renders slot', () => {
    expect(mount(Kbd, { slots: { default: 'Ctrl' } }).element.tagName).toBe('KBD')
  })
})
