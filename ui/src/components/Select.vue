<script setup lang="ts">
import {
  SelectContent,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectPortal,
  SelectRoot,
  SelectTrigger,
  SelectValue,
  SelectViewport,
} from 'reka-ui'
import { Check, ChevronDown } from 'lucide-vue-next'
import { useId } from 'vue'

const model = defineModel<string>()
defineProps<{
  label: string
  options: { value: string; label: string }[]
  placeholder?: string
  disabled?: boolean
}>()
const id = useId()
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <SelectRoot v-model="model" :disabled="disabled">
      <SelectTrigger :id="id" class="trigger">
        <SelectValue :placeholder="placeholder ?? 'Choose...'" />
        <ChevronDown :size="16" aria-hidden="true" />
      </SelectTrigger>
      <SelectPortal>
        <SelectContent class="mg-select-content" position="popper" :side-offset="4">
          <SelectViewport class="viewport">
            <SelectItem v-for="o in options" :key="o.value" :value="o.value" class="item">
              <SelectItemText>{{ o.label }}</SelectItemText>
              <SelectItemIndicator class="ind"><Check :size="14" /></SelectItemIndicator>
            </SelectItem>
          </SelectViewport>
        </SelectContent>
      </SelectPortal>
    </SelectRoot>
  </div>
</template>

<style scoped>
.field {
  display: grid;
  gap: var(--space-2);
}
label {
  font-size: var(--fs-sm);
  font-weight: 500;
}
.trigger {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  cursor: pointer;
}
.trigger[data-placeholder] {
  color: var(--text-muted);
}
.trigger[data-disabled] {
  opacity: 0.55;
  cursor: not-allowed;
}
:global(.mg-select-content) {
  /* Global: reka renders this element without this component's scope id, so a scoped
     rule never matched and the list was unstyled. Positioned so the popper wrapper copies
     the z-index, which keeps it above an open dialog's scrim. */
  position: relative;
  z-index: 60;
  min-width: var(--reka-select-trigger-width);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow-2);
}
.viewport {
  padding: var(--space-1);
}
.item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  height: var(--control-h);
  padding: 0 var(--space-3);
  border-radius: var(--radius-control);
  cursor: pointer;
  outline: none;
}
.item[data-highlighted] {
  background: var(--accent-soft);
}
.ind {
  color: var(--accent);
}
</style>
