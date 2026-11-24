<script setup lang="ts">
// Line-by-line changes of one text file (`diff --patch`). Binary and large
// files have no hunks: say why instead.
import { computed } from 'vue'
import type { FileText } from '@/api'

const props = defineProps<{ text: FileText }>()

/** A side of a hunk header; a side with no lines (added or deleted file) has none to name. */
const range = (start: number, count: number) => (count === 0 ? 'no lines' : `Lines ${start}–${start + count - 1}`)

/** Each line with the line numbers it has on each side. */
const hunks = computed(() =>
  props.text.hunks.map((h) => {
    let o = h.old_start
    let n = h.new_start
    return {
      header: `${range(h.old_start, h.old_lines)} before, ${range(h.new_start, h.new_lines)} after`,
      lines: h.lines.map((l) => ({
        kind: l.kind,
        text: l.text,
        old: l.kind === '+' ? null : o++,
        new: l.kind === '-' ? null : n++,
      })),
    }
  }),
)
const what = (k: string) => (k === '+' ? 'added line' : k === '-' ? 'removed line' : 'unchanged line')
</script>

<template>
  <p v-if="text.kind === 'binary'" class="muted" data-testid="diff-text-binary">Binary file: no line-by-line changes. Sizes and properties are below.</p>
  <p v-else-if="text.kind === 'too_large'" class="muted" data-testid="diff-text-large">Too large to compare line by line (over 8 MB). Sizes and properties are below.</p>
  <p v-else-if="!text.hunks.length" class="muted">No line changes (only the file's metadata differs).</p>
  <div v-else class="hunks" data-testid="diff-hunks">
    <section v-for="(h, i) in hunks" :key="i" class="hunk" :aria-label="h.header">
      <p class="hh">{{ h.header }}</p>
      <div v-for="(l, j) in h.lines" :key="j" class="ln" :class="{ add: l.kind === '+', del: l.kind === '-' }">
        <span class="no" aria-hidden="true">{{ l.old ?? '' }}</span>
        <span class="no" aria-hidden="true">{{ l.new ?? '' }}</span>
        <span class="mk" aria-hidden="true">{{ l.kind }}</span>
        <span class="sr-only">{{ what(l.kind) }}: </span>
        <code class="tx">{{ l.text }}</code>
      </div>
    </section>
    <p v-if="text.truncated" class="muted" role="note">More than 5,000 lines changed; the rest is not shown.</p>
  </div>
</template>

<style scoped>
.muted {
  color: var(--text-muted);
}
.hunks {
  display: grid;
  gap: var(--space-3);
  overflow-x: auto;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
}
.hunk {
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
}
.hh {
  margin: 0;
  padding: var(--space-1) var(--space-3);
  border-bottom: 1px solid var(--border);
  color: var(--text-muted);
  font-family: inherit;
}
.ln {
  display: grid;
  grid-template-columns: 4ch 4ch 2ch 1fr;
  white-space: pre;
}
.ln.add {
  background: var(--added-soft);
}
.ln.del {
  background: var(--deleted-soft);
}
.no {
  padding-right: var(--space-1);
  color: var(--text-muted);
  text-align: right;
  user-select: none;
}
.mk {
  text-align: center;
  user-select: none;
}
.ln.add .mk {
  color: var(--added);
}
.ln.del .mk {
  color: var(--deleted);
}
.tx {
  font-family: inherit;
}
</style>
