<script setup lang="ts">
// Lock state of the selected file, with Lock / Unlock when it is ours to change.
import { computed, onMounted } from 'vue'
import { RouterLink } from 'vue-router'
import { Lock, LockOpen } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import type { InspectFile } from '@/lib/inspect'
import { timeAgo } from '@/lib/persist'
import { useIdentityStore } from '@/stores/identity'
import { useLocksStore } from '@/stores/locks'

const props = defineProps<{ file: InspectFile }>()
const locks = useLocksStore()
const identity = useIdentityStore()

onMounted(() => {
  if (!locks.loaded && !locks.loading) void locks.refresh()
  if (!identity.loaded) void identity.refresh()
})

const held = computed(() => locks.byPath.get(props.file.path))
// Yours when the owner is your account or commit author. With neither known, offer Unlock
// and let the server refuse if it is not ours.
const mine = computed(() => !!held.value && (!locks.names.length || locks.isMine(held.value)))
const here = computed(() => locks.busy === `lock:${props.file.path}` || (!!held.value && locks.busy === `unlock:${held.value.lock_id}`))
const gone = computed(() => props.file.deleted || props.file.status === 'deleted')
</script>

<template>
  <section v-if="!gone" aria-labelledby="insp-lock" data-testid="lock-state">
    <h3 id="insp-lock">Lock</h3>
    <p v-if="!locks.loaded && locks.loading" class="note" role="status">Checking lock status…</p>
    <p v-else-if="!locks.loaded && locks.error" class="note">Lock status is unavailable: {{ locks.error.message }}</p>
    <template v-else-if="held">
      <p class="state"><Lock :size="16" aria-hidden="true" /> Locked by <strong>{{ held.owner }}</strong> · {{ timeAgo(held.created_at * 1000) }}</p>
      <Button v-if="mine" size="sm" :loading="!!here" :disabled="!!locks.busy" @click="locks.unlock(held, false)">Unlock</Button>
      <p v-else class="note">Only {{ held.owner }} can unlock it. A lead can force unlock from <RouterLink to="/team/locks">Locks</RouterLink>.</p>
    </template>
    <template v-else>
      <p class="state"><LockOpen :size="16" aria-hidden="true" /> Not locked</p>
      <Button size="sm" :loading="!!here" :disabled="!!locks.busy" @click="locks.lock(file.path)">Lock this file</Button>
    </template>
    <p v-if="locks.actionError && locks.busy === null" class="note err" role="alert">{{ locks.actionError.message }}</p>
  </section>
</template>

<style scoped>
section {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}
h3 {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-md);
  font-weight: 600;
}
.state {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
}
.note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.err {
  color: var(--danger);
}
a {
  color: var(--accent);
}
</style>
