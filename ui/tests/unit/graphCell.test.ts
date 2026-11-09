import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import GraphCell from '@/views/history/GraphCell.vue'
import { createGraphLayout } from '@/lib/graphLayout'

const rows = createGraphLayout().push([
  { oid: 'm', parents: ['b', 'f'] },
  { oid: 'f', parents: ['a'] },
  { oid: 'b', parents: ['a'] },
  { oid: 'a', parents: [] },
])

describe('GraphCell', () => {
  it('draws one path per segment and one node', () => {
    const w = mount(GraphCell, { props: { row: rows[1]!, height: 36 } })
    expect(w.findAll('path')).toHaveLength(rows[1]!.segments.length)
    expect(w.findAll('circle')).toHaveLength(1)
    expect(w.attributes('aria-hidden')).toBe('true')
  })

  it('curves lane changes and keeps same-lane lines straight', () => {
    const merge = mount(GraphCell, { props: { row: rows[0]!, height: 36 } })
    const ds = merge.findAll('path').map((p) => p.attributes('d'))
    expect(ds.some((d) => d!.includes('C'))).toBe(true) // 0 -> 1 merge edge
    expect(ds.some((d) => d!.includes('V'))).toBe(true) // first-parent line
  })

  it('marks HEAD with an accent ring', () => {
    const w = mount(GraphCell, { props: { row: rows[0]!, height: 36, head: true } })
    expect(w.find('circle').attributes('stroke')).toBe('var(--text)')
  })

  it('sizes the svg to the lanes in use', () => {
    const linear = createGraphLayout().push([{ oid: 'x', parents: [] }])[0]!
    const one = mount(GraphCell, { props: { row: linear, height: 36 } })
    const two = mount(GraphCell, { props: { row: rows[1]!, height: 36 } })
    expect(Number(two.attributes('width'))).toBeGreaterThan(Number(one.attributes('width')))
  })
})
