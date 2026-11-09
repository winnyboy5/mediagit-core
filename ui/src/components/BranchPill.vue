<script setup lang="ts">
import { GitBranch } from 'lucide-vue-next'

defineProps<{ name: string; current?: boolean; lane?: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 }>()
</script>

<template>
  <span class="pill" :class="{ current }" :style="lane !== undefined ? { '--lane': `var(--lane-${lane})` } : undefined">
    <GitBranch :size="14" aria-hidden="true" />
    <span class="n">{{ name }}</span>
    <span v-if="current" class="sr-only">(current branch)</span>
  </span>
</template>

<style scoped>
.pill {
  --lane: var(--text-muted);
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  max-width: 100%;
  height: 24px;
  padding: 0 var(--space-3) 0 var(--space-2);
  border: 1px solid var(--lane);
  border-radius: var(--radius-pill);
  background: var(--surface-2);
  /* ink text: a lane colour alone is only 3:1 */
  color: var(--text);
  font: 500 var(--fs-sm) / 1 var(--font-sans);
}
.pill svg {
  flex: none;
  color: var(--lane);
}
.n {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.current {
  background: var(--accent-soft);
  font-weight: 600;
  box-shadow: 0 0 0 1px var(--lane);
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}
</style>
