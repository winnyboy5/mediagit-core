<script setup lang="ts">
import { CircleDashed, Lock, Minus, Pencil, Plus, TriangleAlert } from 'lucide-vue-next'

export type FileStatus = 'added' | 'modified' | 'deleted' | 'locked' | 'untracked' | 'conflict'

// Letter + icon + hue: status is never colour-only.
const META = {
  added: { letter: 'A', word: 'Added', icon: Plus },
  modified: { letter: 'M', word: 'Modified', icon: Pencil },
  deleted: { letter: 'D', word: 'Deleted', icon: Minus },
  locked: { letter: 'L', word: 'Locked', icon: Lock },
  untracked: { letter: 'U', word: 'Untracked', icon: CircleDashed },
  conflict: { letter: '!', word: 'Conflict', icon: TriangleAlert },
} as const

defineProps<{ status: FileStatus }>()
</script>

<template>
  <span class="badge" :class="status" role="img" :aria-label="META[status].word" :title="META[status].word">
    <component :is="META[status].icon" :size="12" :stroke-width="2.5" aria-hidden="true" />
    <span aria-hidden="true">{{ META[status].letter }}</span>
  </span>
</template>

<style scoped>
.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 3px;
  min-width: 36px;
  height: 22px;
  padding: 0 var(--space-2);
  border-radius: var(--radius-pill);
  font: 600 var(--fs-xs) / 1 var(--font-mono);
}
.added {
  background: var(--added-soft);
  color: var(--added);
}
.modified {
  background: var(--modified-soft);
  color: var(--modified);
}
.deleted {
  background: var(--deleted-soft);
  color: var(--deleted);
}
.locked {
  background: var(--locked-soft);
  color: var(--locked);
}
.untracked {
  background: var(--untracked-soft);
  color: var(--untracked);
}
.conflict {
  background: var(--conflict-soft);
  color: var(--conflict);
}
</style>
