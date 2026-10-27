<script setup lang="ts">
import { ToggleGroupItem, ToggleGroupRoot } from 'reka-ui'

const model = defineModel<string>({ required: true })
defineProps<{ options: { value: string; label: string }[]; label: string }>()

// A segmented control always has one value; reka reports '' on re-click.
function set(v: unknown) {
  if (typeof v === 'string' && v) model.value = v
}
</script>

<template>
  <ToggleGroupRoot type="single" :model-value="model" class="seg" :aria-label="label" @update:model-value="set">
    <ToggleGroupItem v-for="o in options" :key="o.value" :value="o.value" class="opt">{{ o.label }}</ToggleGroupItem>
  </ToggleGroupRoot>
</template>

<style scoped>
.seg {
  display: inline-flex;
  padding: 3px;
  gap: 2px;
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.opt {
  height: calc(var(--control-h-sm) - 2px);
  padding: 0 var(--space-3);
  border: 0;
  border-radius: calc(var(--radius-control) - 2px);
  background: transparent;
  color: var(--text-muted);
  font: 500 var(--fs-sm) / 1 var(--font-sans);
  cursor: pointer;
}
.opt:hover {
  color: var(--text);
}
.opt[data-state='on'] {
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-1);
}
</style>
