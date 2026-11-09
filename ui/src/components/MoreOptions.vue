<script setup lang="ts">
import { ChevronRight } from 'lucide-vue-next'

// Native <details>: keyboard and screen-reader behaviour for free. The panel stays
// mounted while closed so its fields keep their values.
withDefaults(defineProps<{ label?: string }>(), { label: 'More options' })
</script>

<template>
  <details class="more">
    <summary>
      <ChevronRight :size="16" aria-hidden="true" class="chev" />
      {{ label }}
    </summary>
    <div class="panel"><slot /></div>
  </details>
</template>

<style scoped>
.more {
  border-top: 1px solid var(--border);
  padding-top: var(--space-3);
}
summary {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-2);
  margin-left: calc(var(--space-2) * -1);
  border-radius: var(--radius-control);
  color: var(--text-muted);
  font-size: var(--fs-sm);
  font-weight: 500;
  cursor: pointer;
  list-style: none;
}
summary::-webkit-details-marker {
  display: none;
}
summary:hover {
  color: var(--text);
}
summary:focus-visible {
  outline: 2px solid var(--focus, var(--accent));
  outline-offset: 2px;
}
.chev {
  transition: transform var(--dur) var(--ease);
}
.more[open] .chev {
  transform: rotate(90deg);
}
.panel {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-1) var(--space-1);
  max-height: min(50vh, 28rem);
  overflow-y: auto;
  contain: paint;
}
</style>
