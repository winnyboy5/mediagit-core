<script setup lang="ts">
import { CircleAlert } from 'lucide-vue-next'
import Button from './Button.vue'

defineProps<{ code: string; message: string }>()
const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="err" role="alert">
    <span class="ico" aria-hidden="true"><CircleAlert :size="28" /></span>
    <h3>Something went wrong</h3>
    <p>{{ message }}</p>
    <code class="mono">{{ code }}</code>
    <Button variant="secondary" @click="emit('retry')">Retry</Button>
  </div>
</template>

<style scoped>
.err {
  display: grid;
  justify-items: center;
  gap: var(--space-2);
  padding: var(--space-12) var(--space-6);
  text-align: center;
}
.ico {
  display: grid;
  place-items: center;
  width: 56px;
  height: 56px;
  margin-bottom: var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--danger-soft);
  color: var(--danger);
}
h3 {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
p {
  max-width: 52ch;
  margin: 0;
  color: var(--text-muted);
  text-wrap: balance;
}
code {
  padding: 2px var(--space-2);
  border-radius: var(--radius-control);
  background: var(--surface-2);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.err :deep(.btn) {
  margin-top: var(--space-3);
}
</style>
