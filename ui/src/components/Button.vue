<script setup lang="ts">
import { LoaderCircle } from 'lucide-vue-next'

withDefaults(
  defineProps<{
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger'
    size?: 'md' | 'sm'
    loading?: boolean
    disabled?: boolean
    type?: 'button' | 'submit'
  }>(),
  { variant: 'secondary', size: 'md', type: 'button' },
)
</script>

<template>
  <button
    :type="type"
    class="btn"
    :class="[variant, size]"
    :disabled="disabled || loading"
    :aria-busy="loading || undefined"
  >
    <LoaderCircle v-if="loading" class="spin" :size="16" aria-hidden="true" />
    <slot />
  </button>
</template>

<style scoped>
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  height: var(--control-h);
  padding: 0 var(--space-4);
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  font: 500 var(--fs-md) / 1 var(--font-sans);
  white-space: nowrap;
  cursor: pointer;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease), color var(--dur) var(--ease);
}
.btn.sm {
  height: var(--control-h-sm);
  padding: 0 var(--space-3);
  font-size: var(--fs-sm);
}
.btn:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.btn[aria-busy='true'] {
  cursor: progress;
  opacity: 0.85;
}
.primary {
  background: var(--accent);
  color: var(--on-accent);
}
.primary:hover:not(:disabled) {
  background: var(--accent-hover);
}
.secondary {
  background: var(--surface);
  color: var(--text);
  border-color: var(--border-strong);
}
.secondary:hover:not(:disabled) {
  background: var(--surface-2);
}
.ghost {
  background: transparent;
  color: var(--text);
}
.ghost:hover:not(:disabled) {
  background: var(--surface-2);
}
.danger {
  background: var(--danger-soft);
  color: var(--danger);
  border-color: var(--danger);
}
.danger:hover:not(:disabled) {
  background: var(--danger);
  color: var(--surface);
}
.spin {
  animation: spin 900ms linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .spin {
    animation: none;
  }
}
</style>
