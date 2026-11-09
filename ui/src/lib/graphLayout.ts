// Lane layout for the history graph (V3). Pure and incremental: commits arrive
// page by page in display order (newest first, parents after children) and
// lanes never move once assigned, so appending a page never reflows rows
// already on screen.

export interface GraphCommit {
  oid: string
  parents: string[]
}

/** One line inside a row, in lane units. `top` runs from the row's top edge to
 *  the node's centre line, `bottom` from the centre line to the bottom edge. */
export interface GraphSegment {
  from: number
  to: number
  half: 'top' | 'bottom'
}

export interface GraphRow {
  /** Lane the commit's node sits in. */
  col: number
  /** Lanes occupied in this row (for sizing the SVG). */
  width: number
  segments: GraphSegment[]
}

export interface GraphLayout {
  push(commits: GraphCommit[]): GraphRow[]
}

/** Lane colour index: lane 0 is ink (HEAD), the rest cycle 1..7. */
export const laneIndex = (col: number) => (col === 0 ? 0 : 1 + ((col - 1) % 7))

/** `headOid` reserves lane 0 for the current branch: its first-parent chain claims it. */
export function createGraphLayout(headOid?: string | null): GraphLayout {
  // lanes[i] = oid the line in lane i is heading towards, or null if free.
  const lanes: (string | null)[] = headOid ? [headOid] : []
  // The seeded lane draws nothing until HEAD itself arrives (nothing comes from above it).
  let headPending = !!headOid

  const used = () => {
    let n = lanes.length
    while (n > 0 && lanes[n - 1] === null) n--
    return n
  }
  const claim = (oid: string) => {
    const free = lanes.indexOf(null)
    if (free >= 0) {
      lanes[free] = oid
      return free
    }
    lanes.push(oid)
    return lanes.length - 1
  }

  function place({ oid, parents }: GraphCommit): GraphRow {
    const segments: GraphSegment[] = []
    const widthBefore = used()

    let col = lanes.indexOf(oid)
    // Every line arriving from above either continues straight down or, if it
    // was heading for this commit, bends into the node and its lane frees up.
    lanes.forEach((target, j) => {
      if (target === null) return
      if (headPending && j === 0) {
        if (target === oid) headPending = false
        return
      }
      if (target === oid) {
        segments.push({ from: j, to: col, half: 'top' })
        if (j !== col) lanes[j] = null
      } else {
        segments.push({ from: j, to: j, half: 'top' })
      }
    })
    if (col < 0) col = claim(oid) // a branch tip: nothing arrives from above

    // The first parent continues in this commit's lane; merge parents open (or
    // join) another lane.
    lanes[col] = parents[0] ?? null
    const parentLanes = new Set<number>()
    const joined = new Set<number>() // merge parent already had a lane: it keeps its own line too
    if (parents[0] !== undefined) parentLanes.add(col)
    for (const p of parents.slice(1)) {
      const existing = lanes.indexOf(p)
      if (existing >= 0) joined.add(existing)
      parentLanes.add(existing >= 0 ? existing : claim(p))
    }

    lanes.forEach((target, k) => {
      if (target === null || (headPending && k === 0)) return
      if (joined.has(k) || !parentLanes.has(k)) segments.push({ from: k, to: k, half: 'bottom' })
      if (parentLanes.has(k)) segments.push({ from: col, to: k, half: 'bottom' })
    })

    return { col, width: Math.max(widthBefore, used(), col + 1), segments }
  }

  return { push: (commits) => commits.map(place) }
}
