<script setup lang="ts">
import { CheckboxIndicator, CheckboxRoot } from 'reka-ui'
import { Check, Minus } from 'lucide-vue-next'

const model = defineModel<boolean | 'indeterminate'>({ default: false })
defineProps<{ label?: string; disabled?: boolean; ariaLabel?: string; id?: string; ariaDescribedby?: string }>()
</script>

<template>
  <label class="check" :class="{ disabled }">
    <CheckboxRoot v-model="model" class="box" :disabled="disabled" :id="id" :aria-label="ariaLabel" :aria-describedby="ariaDescribedby">
      <CheckboxIndicator class="mark">
        <Minus v-if="model === 'indeterminate'" :size="14" :stroke-width="3" />
        <Check v-else :size="14" :stroke-width="3" />
      </CheckboxIndicator>
    </CheckboxRoot>
    <span v-if="label">{{ label }}</span>
  </label>
</template>

<style scoped>
.check {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  cursor: pointer;
}
.check.disabled {
  color: var(--text-muted);
  cursor: not-allowed;
}
.check.disabled .box {
  opacity: 0.55;
}
.box {
  display: inline-grid;
  place-items: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: 1.5px solid var(--border-strong);
  border-radius: 5px;
  background: var(--surface);
  color: var(--on-accent);
  cursor: inherit;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
}
.box[data-state='checked'],
.box[data-state='indeterminate'] {
  background: var(--accent);
  border-color: var(--accent);
}
.mark {
  display: grid;
  place-items: center;
}
</style>
