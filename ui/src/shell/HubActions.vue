<script setup lang="ts">
// Get latest / Send: part of the daily loop, so they sit in every work hub's header.
import { computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ArrowDown, ArrowUp } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore } from '@/stores/sync'

const repo = useRepoStore()
const sync = useSyncStore()
const router = useRouter()
const route = useRoute()

onMounted(() => {
  if (repo.path && !sync.loaded) void sync.refresh()
})
watch(
  () => repo.path,
  (p) => p && void sync.refresh(),
)

const up = computed(() => repo.status?.branch.upstream)
const behind = computed(() => up.value?.behind ?? 0)
const ahead = computed(() => up.value?.ahead ?? 0)
const none = computed(() => !repo.path || !sync.remote)

function send() {
  // Overwriting or deleting on the remote is confirmed on the Sync page, where its options are.
  if (!sync.pushNeedsConfirm) return void sync.run('push')
  sync.confirmPushOpen = true
  if (route.path !== '/branches/sync') void router.push('/branches/sync')
}
</script>

<template>
  <div class="acts" role="group" aria-label="Share work">
    <Button :disabled="sync.busy || none" :variant="behind > 0 && ahead === 0 ? 'primary' : 'secondary'" @click="sync.run('pull')">
      <ArrowDown :size="16" aria-hidden="true" /> Get latest<span v-if="behind" class="n"> {{ behind }}</span>
    </Button>
    <Button :disabled="sync.busy || none || sync.pushDeleteNeedsName" :variant="ahead > 0 ? 'primary' : 'secondary'" @click="send">
      <ArrowUp :size="16" aria-hidden="true" /> Send<span v-if="ahead" class="n"> {{ ahead }}</span>
    </Button>
  </div>
</template>

<style scoped>
.acts {
  display: flex;
  gap: var(--space-2);
}
.n {
  font-weight: 600;
}
</style>
