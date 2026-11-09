<script setup lang="ts">
// Shown only while something runs, and for 10 seconds after it ends. Opens the Activity drawer.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Check, TriangleAlert } from 'lucide-vue-next'
import ProgressBar from '@/components/ProgressBar.vue'
import { formatBytes, formatEta, formatRate } from '@/components/format'
import { stagingStats } from '@/lib/staging'
import { primaryBar, useOpsStore, type Op } from '@/stores/ops'
import { useShellStore } from '@/stores/shell'

const AFTER_MS = 10_000
const ops = useOpsStore()
const shell = useShellStore()

const op = computed(() => ops.current)
const bar = computed(() => (op.value ? primaryBar(op.value) : undefined))
const pct = computed(() => (bar.value?.len ? Math.min(100, Math.round((bar.value.pos / bar.value.len) * 100)) : null))
// Staging reports files and speed too.
const stats = computed(() => (op.value?.kind === 'add' ? stagingStats(op.value) : null))
const others = computed(() => ops.running.length - (op.value ? 1 : 0) + ops.queued.length)

// The last op to end stays for a while so a failure is not missed.
const ended = ref<Op>()
let timer: ReturnType<typeof setTimeout> | undefined
watch(
  () => ops.finished[0]?.id,
  () => {
    ended.value = ops.finished[0]
    clearTimeout(timer)
    timer = setTimeout(() => (ended.value = undefined), AFTER_MS)
  },
)
onBeforeUnmount(() => clearTimeout(timer))

const show = computed(() => !!op.value || !!ended.value)
</script>

<template>
  <button
    v-if="show"
    type="button"
    class="pill"
    data-activity-pill
    aria-haspopup="dialog"
    :aria-expanded="shell.activityOpen"
    @click="shell.activityOpen = !shell.activityOpen"
  >
    <template v-if="op">
      <strong>{{ op.label }}</strong>
      <span v-if="pct !== null">{{ pct }}%</span>
      <span v-if="stats?.files" class="phase" data-testid="pill-files">{{ stats.files.done }}{{ stats.files.total != null ? ` of ${stats.files.total}` : '' }} files</span>
      <span v-if="stats?.bytes" class="phase" data-testid="pill-bytes">{{ formatBytes(stats.bytes.done) }}{{ stats.bytes.total != null ? ` of ${formatBytes(stats.bytes.total)}` : '' }}</span>
      <span v-if="stats?.rate != null" class="phase" data-testid="pill-rate">{{ formatRate(stats.rate) }}</span>
      <span v-if="stats?.eta != null" class="phase" data-testid="pill-eta">{{ formatEta(stats.eta) }} left</span>
      <span v-else class="phase" aria-live="polite">{{ op.phase }}</span>
      <span v-if="others" class="phase">+{{ others }} more</span>
      <ProgressBar class="mini" size="sm" :label="op.label" :value="bar?.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
    </template>
    <template v-else-if="ended">
      <TriangleAlert v-if="ended.status === 'failed'" :size="16" aria-hidden="true" />
      <Check v-else :size="16" aria-hidden="true" />
      <strong>{{ ended.label }}</strong>
      <span>{{ ended.status === 'failed' ? 'failed' : ended.status === 'cancelled' ? 'cancelled' : 'done' }}</span>
    </template>
  </button>
</template>

<style scoped>
.pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  max-width: 460px;
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-pill);
  background: var(--surface);
  color: var(--text);
  font: 400 var(--fs-sm) / 1 var(--font-sans);
  white-space: nowrap;
  cursor: pointer;
}
.pill:hover {
  background: var(--surface-2);
}
.pill strong {
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 600;
}
.phase {
  color: var(--text-muted);
}
.mini {
  width: 56px;
  flex: none;
}
</style>
