import { formatBytes } from '@/components/format'
import { flattenInfo } from './inspect'

export interface MetaRow {
  label: string
  before: string
  after: string
  changed: boolean
}

const DASH = '–'

/**
 * Before/after property rows: the sizes first, then whatever the media parser reported
 * for either side. A property one side lacks shows a dash and counts as changed.
 */
export function metaRows(
  before: Record<string, unknown> | undefined,
  after: Record<string, unknown> | undefined,
  sizes: { before: number | null; after: number | null },
): MetaRow[] {
  const size = (n: number | null) => (n == null ? DASH : formatBytes(n))
  const rows: MetaRow[] = [{ label: 'Size', before: size(sizes.before), after: size(sizes.after), changed: sizes.before !== sizes.after }]
  const a = new Map((before ? flattenInfo(before) : []).map((r) => [r.label, r.value]))
  const b = new Map((after ? flattenInfo(after) : []).map((r) => [r.label, r.value]))
  for (const label of new Set([...a.keys(), ...b.keys()])) {
    if (label === 'File size') continue // the Size row above already says it
    const x = a.get(label)
    const y = b.get(label)
    rows.push({ label, before: x ?? DASH, after: y ?? DASH, changed: x !== y })
  }
  return rows
}
