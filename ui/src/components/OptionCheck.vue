<script setup lang="ts">
import { useId } from 'vue'
import Checkbox from './Checkbox.vue'

// One option: plain-words label, what it does, and the CLI flag as secondary text.
// `danger` marks an option that can lose work; it is styled, never a default.
const model = defineModel<boolean>({ default: false })
defineProps<{ label: string; flag?: string; hint?: string; danger?: boolean; disabled?: boolean }>()
const id = useId()
</script>

<template>
  <div class="opt" :class="{ danger }">
    <Checkbox :id="id" v-model="model" :aria-label="label" :aria-describedby="hint ? id + '-hint' : undefined" :disabled="disabled" />
    <div class="txt">
      <label :for="id" class="lbl">{{ label }} <code v-if="flag" class="mono flag">{{ flag }}</code></label>
      <span v-if="hint" :id="id + '-hint'" class="hint">{{ hint }}</span>
    </div>
  </div>
</template>

<style scoped>
.opt {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
.txt {
  display: grid;
  gap: 2px;
}
.lbl {
  font-size: var(--fs-md);
  cursor: pointer;
}
.flag {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  white-space: nowrap;
}
.hint {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.danger .lbl {
  color: var(--danger);
  font-weight: 500;
}
</style>
