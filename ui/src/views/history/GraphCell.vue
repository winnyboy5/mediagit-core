<script setup lang="ts">
// One row of the history graph: lane lines in and out, plus the commit node.
// Geometry comes from lib/graphLayout; colours from the --lane-0..7 tokens.
import { computed } from 'vue'
import { laneIndex, type GraphRow } from '@/lib/graphLayout'

const props = defineProps<{ row: GraphRow; height: number; head?: boolean }>()

const LANE = 14 // px between lane centres
const PAD = 8

const x = (lane: number) => PAD + lane * LANE
const laneColor = (lane: number) => `var(--lane-${laneIndex(lane)})`
const width = computed(() => PAD * 2 + Math.max(props.row.width - 1, 0) * LANE)
const mid = computed(() => props.height / 2)

// Straight when the lane does not change, otherwise a smooth S-curve.
function path(from: number, to: number, y1: number, y2: number) {
  const x1 = x(from)
  const x2 = x(to)
  if (x1 === x2) return `M${x1},${y1}V${y2}`
  const ym = (y1 + y2) / 2
  return `M${x1},${y1}C${x1},${ym} ${x2},${ym} ${x2},${y2}`
}

const lines = computed(() =>
  props.row.segments.map((s) => {
    const [y1, y2] = s.half === 'top' ? [0, mid.value] : [mid.value, props.height]
    // A line takes the colour of the lane it ends in on the way down to a
    // parent, and of the lane it starts in when it arrives from a child.
    const lane = s.half === 'top' ? s.from : s.to
    return { d: path(s.from, s.to, y1, y2), color: laneColor(lane) }
  }),
)
</script>

<template>
  <svg :width="width" :height="height" class="graph-cell" aria-hidden="true">
    <path v-for="(l, i) in lines" :key="i" :d="l.d" :stroke="l.color" fill="none" stroke-width="2" />
    <circle
      :cx="x(row.col)"
      :cy="mid"
      :r="head ? 6 : 5"
      fill="var(--surface)"
      :stroke="head ? 'var(--text)' : laneColor(row.col)"
      :stroke-width="head ? 3 : 2.5"
    />
  </svg>
</template>

<style scoped>
.graph-cell {
  display: block;
  flex: none;
  overflow: visible;
}
</style>
