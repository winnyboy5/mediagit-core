<script setup lang="ts">
import { CircleDashed, Lock, Minus, Pencil, Plus, TriangleAlert } from 'lucide-vue-next'

export type FileStatus = 'added' | 'modified' | 'deleted' | 'locked' | 'untracked' | 'conflict'

// Icon + plain word + hue: status is never colour-only, and never a letter code.
const META = {
  added: { word: 'New', icon: Plus },
  modified: { word: 'Edited', icon: Pencil },
  deleted: { word: 'Deleted', icon: Minus },
  locked: { word: 'Locked', icon: Lock },
  untracked: { word: 'New', icon: CircleDashed },
  conflict: { word: 'Conflict', icon: TriangleAlert },
} as const

defineProps<{ status: FileStatus }>()
</script>

<template>
  <span class="badge" :class="status">
    <component :is="META[status].icon" :size="12" :stroke-width="2.5" aria-hidden="true" />
    {{ META[status].word }}
  </span>
</template>

<style scoped>
.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: 22px;
  padding: 0 var(--space-2);
  border-radius: var(--radius-pill);
  font: 600 var(--fs-xs) / 1 var(--font-sans);
  white-space: nowrap;
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
