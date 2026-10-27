<script setup lang="ts">
withDefaults(
  defineProps<{
    label: string
    variant?: 'ghost' | 'secondary'
    size?: 'md' | 'sm'
    disabled?: boolean
    pressed?: boolean
  }>(),
  { variant: 'ghost', size: 'md' },
)
</script>

<template>
  <button
    type="button"
    class="icon-btn"
    :class="[variant, size]"
    :aria-label="label"
    :aria-pressed="pressed"
    :disabled="disabled"
  >
    <slot />
  </button>
</template>

<style scoped>
.icon-btn {
  display: inline-grid;
  place-items: center;
  width: var(--control-h);
  height: var(--control-h);
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.icon-btn.sm {
  width: var(--control-h-sm);
  height: var(--control-h-sm);
}
.icon-btn.secondary {
  background: var(--surface);
  border-color: var(--border-strong);
  color: var(--text);
}
.icon-btn:hover:not(:disabled) {
  background: var(--surface-2);
  color: var(--text);
}
.icon-btn[aria-pressed='true'] {
  background: var(--accent-soft);
  color: var(--text);
}
.icon-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
