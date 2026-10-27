<script setup lang="ts">
import { computed, useId } from 'vue'

const model = defineModel<string>({ default: '' })
const props = defineProps<{
  label: string
  placeholder?: string
  hint?: string
  error?: string
  rows?: number
  disabled?: boolean
}>()
const id = useId()
const note = computed(() => props.error ?? props.hint)
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <textarea
      :id="id"
      v-model="model"
      :class="{ invalid: !!error }"
      :rows="rows ?? 4"
      :placeholder="placeholder"
      :disabled="disabled"
      :aria-invalid="error ? true : undefined"
      :aria-describedby="note ? id + '-note' : undefined"
    />
    <p v-if="note" :id="id + '-note'" class="note" :class="{ err: !!error }">{{ note }}</p>
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
textarea {
  padding: var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  line-height: 1.5;
  resize: vertical;
}
textarea::placeholder {
  color: var(--text-muted);
}
textarea.invalid {
  border-color: var(--danger);
}
textarea:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}
.note {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}
.note.err {
  color: var(--danger);
}
</style>
