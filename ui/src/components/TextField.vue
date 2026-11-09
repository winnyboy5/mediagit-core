<script setup lang="ts">
import { computed, useId } from 'vue'

const model = defineModel<string>({ default: '' })
const props = defineProps<{
  label: string
  type?: string
  placeholder?: string
  hint?: string
  error?: string
  disabled?: boolean
  mono?: boolean
  flag?: string
}>()
const id = useId()
const note = computed(() => props.error ?? props.hint)
const noted = computed(() => !!note.value || (!!props.flag && !props.error))
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <input
      :id="id"
      v-model="model"
      :class="{ mono, invalid: !!error }"
      :type="type ?? 'text'"
      :placeholder="placeholder"
      :disabled="disabled"
      :aria-invalid="error ? true : undefined"
      :aria-describedby="noted ? id + '-note' : undefined"
    />
    <p v-if="noted" :id="id + '-note'" class="note" :class="{ err: !!error }">{{ note }}<template v-if="flag && !error"> <code class="mono flag">{{ flag }}</code></template></p>
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
input {
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
}
input.mono {
  font-family: var(--font-mono);
}
input::placeholder {
  color: var(--text-muted);
  font-family: var(--font-sans, inherit);
  font-style: italic;
}
input.invalid {
  border-color: var(--danger);
}
input:disabled {
  background: var(--surface-2);
  color: var(--text-muted);
  border-color: var(--border);
  cursor: not-allowed;
}
.note {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}
.flag {
  /* Vue drops the space before this inline element; keep the gap here. */
  margin-inline-start: 0.35em;
  font-size: var(--fs-sm);
  white-space: nowrap;
}
.note.err {
  color: var(--danger);
}
</style>
