import { describe, expect, it } from 'vitest'
import { createGraphLayout, type GraphCommit, type GraphRow } from '@/lib/graphLayout'

const c = (oid: string, ...parents: string[]): GraphCommit => ({ oid, parents })
const layout = (...commits: GraphCommit[]): GraphRow[] => createGraphLayout().push(commits)
const cols = (rows: GraphRow[]) => rows.map((r) => r.col)

describe('graph layout', () => {
  it('keeps a linear history in one lane', () => {
    const rows = layout(c('c', 'b'), c('b', 'a'), c('a'))
    expect(cols(rows)).toEqual([0, 0, 0])
    expect(rows.every((r) => r.width === 1)).toBe(true)
    // middle row: line in from above, line out below, both lane 0
    expect(rows[1]!.segments).toEqual([
      { from: 0, to: 0, half: 'top' },
      { from: 0, to: 0, half: 'bottom' },
    ])
    // root: nothing below; tip: nothing above
    expect(rows[2]!.segments.some((s) => s.half === 'bottom')).toBe(false)
    expect(rows[0]!.segments.some((s) => s.half === 'top')).toBe(false)
  })

  it('opens a lane for a merged-in branch and closes it at the fork point', () => {
    // m merges f into main; f and b both descend from a.
    const rows = layout(c('m', 'b', 'f'), c('f', 'a'), c('b', 'a'), c('a'))
    expect(cols(rows)).toEqual([0, 1, 0, 0])
    expect(rows[0]!.segments).toContainEqual({ from: 0, to: 1, half: 'bottom' }) // merge edge
    expect(rows[1]!.segments).toContainEqual({ from: 0, to: 0, half: 'top' }) // main passes f's row
    expect(rows[1]!.segments).toContainEqual({ from: 0, to: 0, half: 'bottom' })
    // at a, lane 1 converges into lane 0
    expect(rows[3]!.segments).toContainEqual({ from: 1, to: 0, half: 'top' })
    expect(rows[3]!.width).toBe(2)
  })

  it('places two tips that share a parent side by side', () => {
    const rows = layout(c('x', 'a'), c('y', 'a'), c('a'))
    expect(cols(rows)).toEqual([0, 1, 0])
    expect(rows[2]!.segments).toContainEqual({ from: 1, to: 0, half: 'top' })
  })

  it('keeps the line of a lane that a merge joins', () => {
    // y already heads for a in lane 0; m (lane 1) merges a in as its 2nd parent.
    const rows = layout(c('y', 'a'), c('m', 'b', 'a'), c('b', 'a'), c('a'))
    const bottom = rows[1]!.segments.filter((s) => s.half === 'bottom')
    expect(bottom).toContainEqual({ from: 0, to: 0, half: 'bottom' }) // lane 0 continues
    expect(bottom).toContainEqual({ from: 1, to: 0, half: 'bottom' }) // merge edge
    expect(bottom).toContainEqual({ from: 1, to: 1, half: 'bottom' }) // first parent b
  })

  it('handles an octopus merge with three parents', () => {
    const rows = layout(c('o', 'a', 'b', 'd'), c('d', 'r'), c('b', 'r'), c('a', 'r'), c('r'))
    expect(rows[0]!.segments.filter((s) => s.half === 'bottom').map((s) => s.to)).toEqual([0, 1, 2])
    expect(cols(rows)).toEqual([0, 2, 1, 0, 0])
  })

  it('reuses a freed lane instead of growing the graph', () => {
    // f1 joins at a1; f2 opens later and must take the freed lane 1, not lane 2.
    const rows = layout(
      c('m2', 'a2', 'f2'),
      c('f2', 'a2'),
      c('a2', 'm1'),
      c('m1', 'a1', 'f1'),
      c('f1', 'a1'),
      c('a1'),
    )
    expect(Math.max(...rows.map((r) => r.width))).toBe(2)
  })

  it('keeps lanes stable across pages', () => {
    const all = [c('m', 'b', 'f'), c('f', 'a'), c('b', 'a'), c('a')]
    const whole = createGraphLayout().push(all)
    const paged = createGraphLayout()
    const rows = [...paged.push(all.slice(0, 2)), ...paged.push(all.slice(2))]
    expect(rows).toEqual(whole)
  })

  it('continues lanes whose parent is on a later page', () => {
    const rows = createGraphLayout().push([c('b', 'a')])
    expect(rows[0]!.segments).toContainEqual({ from: 0, to: 0, half: 'bottom' })
  })

  it('lays out 10k commits fast', () => {
    const commits: GraphCommit[] = []
    for (let i = 10000; i > 0; i--) {
      // every 10th commit merges a short side branch
      commits.push(i % 10 === 0 ? c(`m${i}`, `m${i - 1}`, `s${i}`) : c(`m${i}`, `m${i - 1}`))
      if (i % 10 === 0) commits.push(c(`s${i}`, `m${i - 1}`))
    }
    const t0 = performance.now()
    const rows = createGraphLayout().push(commits)
    expect(performance.now() - t0).toBeLessThan(50)
    expect(rows).toHaveLength(commits.length)
  })
})

describe('graph layout with HEAD reserved in lane 0', () => {
  it('keeps the HEAD chain in lane 0 and draws nothing above HEAD for newer branches', () => {
    // t is a newer tip on another branch; h is HEAD; both descend from a.
    const rows = createGraphLayout('h').push([c('t', 'a'), c('h', 'a'), c('a')])
    expect(cols(rows)).toEqual([1, 0, 0])
    // lane 0 is reserved but silent on the first row, and nothing arrives from above HEAD
    expect(rows[0]!.segments.some((s) => s.from === 0 && s.to === 0)).toBe(false)
    expect(rows[1]!.segments.some((s) => s.half === 'top' && s.from === 0 && s.to === 0)).toBe(false)
    expect(rows[1]!.segments).toContainEqual({ from: 1, to: 1, half: 'top' })
    // the other branch joins HEAD's lane at the fork point
    expect(rows[2]!.segments).toContainEqual({ from: 1, to: 0, half: 'top' })
  })

  it('matches the unseeded layout when HEAD is the newest commit', () => {
    const commits = [c('h', 'b'), c('b', 'a'), c('a')]
    expect(createGraphLayout('h').push(commits)).toEqual(createGraphLayout().push(commits))
  })
})
