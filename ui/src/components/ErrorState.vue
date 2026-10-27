<script setup lang="ts">
import { computed, inject } from 'vue'
import { routerKey } from 'vue-router'
import { CircleAlert } from 'lucide-vue-next'
import Button from './Button.vue'

const props = defineProps<{ code: string; message: string }>()
const emit = defineEmits<{ retry: [] }>()
const router = inject(routerKey, null)

// The first sentence names the problem; any rest is the explanation.
const parts = computed(() => {
  const m = props.message.trim()
  const i = m.search(/[.!?](\s|$)/)
  const head = (i < 0 ? m : m.slice(0, i)).trim()
  return { title: head || 'Something went wrong', rest: i < 0 ? '' : m.slice(i + 1).trim() }
})
// "error" / "unknown" say nothing a person can act on.
const showCode = computed(() => !!props.code && !/^(error|unknown)$/i.test(props.code))
const noRemote = computed(() => /no remote|remote .*(not (found|configured|set))|no such remote/i.test(props.message))
// Retrying cannot fix a missing repo, a missing remote or a refusal.
const canRetry = computed(() => !noRemote.value && !/not a mediagit repository|protected|refused|permission/i.test(props.message))
</script>

<template>
  <div class="err" role="alert">
    <span class="ico" aria-hidden="true"><CircleAlert :size="28" /></span>
    <h3>{{ parts.title }}</h3>
    <p v-if="parts.rest">{{ parts.rest }}</p>
    <code v-if="showCode" class="mono">{{ code }}</code>
    <Button v-if="noRemote && router" variant="secondary" @click="router.push('/sync')">Add remote</Button>
    <Button v-else-if="canRetry" variant="secondary" @click="emit('retry')">Retry</Button>
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
