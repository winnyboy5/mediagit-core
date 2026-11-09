<script setup lang="ts">
import { Tag } from 'lucide-vue-next'

defineProps<{ name: string }>()
</script>

<template>
  <span class="pill">
    <Tag :size="14" aria-hidden="true" />
    <span class="n">{{ name }}</span>
  </span>
</template>

<style scoped>
.pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  max-width: 100%;
  height: 24px;
  padding: 0 var(--space-3) 0 var(--space-2);
  border-radius: var(--radius-pill);
  border: 1px solid var(--border-strong);
  background: transparent;
  color: var(--text);
  font: 500 var(--fs-sm) / 1 var(--font-sans);
}
.pill svg {
  flex: none;
}
.n {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
