<script setup lang="ts">
import { computed, inject } from 'vue'
import { getActivePinia } from 'pinia'
import { routerKey } from 'vue-router'
import { CircleAlert } from 'lucide-vue-next'
import { useRepoStore } from '@/stores/repo'
import { useUnlockStore } from '@/stores/unlock'
import { plainError } from '@/lib/errorText'
import Button from './Button.vue'

const props = defineProps<{ code: string; message: string; details?: string }>()
const emit = defineEmits<{ retry: [] }>()
const router = inject(routerKey, null)

// The first sentence names the problem; any rest is the explanation.
// After a Cancel the error is "cancelled": on a repository left locked, say it is locked. Any other error keeps its own text.
const repoLocked = computed(() => {
  if (!getActivePinia()) return false
  const path = useRepoStore().path
  return !!path && useUnlockStore().locked.has(path)
})
const locked = computed(() => props.code === 'needs_input' || (props.code === 'cancelled' && repoLocked.value))
// Stores are touched only when asked, so this stays a plain component everywhere else.
async function unlockThenRetry() {
  const path = useRepoStore().path
  if (path && (await useUnlockStore().unlockRepo(path).catch(() => false))) emit('retry')
}
const view = computed(() => plainError(props.code, props.message, props.details))
const parts = computed(() => {
  if (locked.value) return { title: 'This repository is locked', rest: 'Its files are encrypted. Unlock it with its passphrase to continue.' }
  const m = view.value.text
  const i = m.search(/[.!?](\s|$)/)
  const head = (i < 0 ? m : m.slice(0, i)).trim()
  return { title: head || 'Something went wrong', rest: i < 0 ? '' : m.slice(i + 1).trim() }
})
// The code is often just "error"; the raw text, kept under Details, says what failed.
const raw = computed(() => `${props.message}
${view.value.details}`)
const noRemote = computed(() => /no remote|remote .*(not (found|configured|set))|no such remote/i.test(raw.value))
// Retrying cannot fix a missing repo, a missing remote or a refusal; the table decides the rest.
const canRetry = computed(() => view.value.retry && !noRemote.value && !/not a mediagit repository|protected|refused/i.test(raw.value))
</script>

<template>
  <div class="err" role="alert">
    <span class="ico" aria-hidden="true"><CircleAlert :size="28" /></span>
    <h3>{{ parts.title }}</h3>
    <p v-if="parts.rest">{{ parts.rest }}</p>
    <details v-if="view.details" class="more">
      <summary>Details</summary>
      <pre class="mono">{{ view.details }}</pre>
    </details>
    <Button v-if="locked" variant="primary" @click="unlockThenRetry">Unlock…</Button>
    <Button v-else-if="noRemote && router" variant="secondary" @click="router.push('/branches/sync')">Add remote</Button>
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
.more {
  max-width: 52ch;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.more summary {
  cursor: pointer;
}
.more pre {
  margin: var(--space-2) 0 0;
  text-align: left;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.err :deep(.btn) {
  margin-top: var(--space-3);
}
</style>
