<script setup lang="ts">
import { computed, ref } from 'vue'
import { CircleCheck, TriangleAlert } from 'lucide-vue-next'
import { formatBytes } from '@/components/format'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import { timeAgo } from '@/lib/persist'
import { primaryBar } from '@/stores/ops'
import { useMaintenanceStore, type Task } from '@/stores/maintenance'
import { useRepoStore } from '@/stores/repo'

const m = useMaintenanceStore()
const repo = useRepoStore()

const preview = ref<boolean | 'indeterminate'>(true)
const deep = ref<boolean | 'indeterminate'>(false)
const quick = ref<boolean | 'indeterminate'>(false)
const confirmOpen = ref(false)

const bar = computed(() => (m.running ? primaryBar(m.running) : undefined))
const last = (t: Task) => {
  const at = repo.path ? m.last[repo.path]?.[t] : undefined
  return at ? `Last run ${timeAgo(at)}` : 'Never run'
}
const n = (x: number) => x.toLocaleString('en-US')
const warn = (sev: string) => /^warn/i.test(sev)
const running = (t: Task) => m.task === t && !!m.running

function runGc() {
  if (preview.value === true) void m.runGc(true, false)
  else confirmOpen.value = true
}
</script>

<template>
  <section class="mt" :aria-busy="m.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Maintenance</h1>
        <p>clean up and check the repository</p>
      </div>
    </header>

    <div v-if="m.error" class="banner" role="alert">
      <span class="chip bad">Error</span>
      <span>{{ m.error.message }}</span>
      <Button size="sm" variant="ghost" @click="m.error = null">Dismiss</Button>
    </div>

    <div class="grid">
      <!-- Clean up -->
      <section class="card" aria-labelledby="gc-h" data-task="gc">
        <h2 id="gc-h">Clean up storage</h2>
        <p class="muted">Remove data no version needs.</p>
        <Checkbox v-model="preview" label="Preview only" :disabled="m.busy" />
        <Button :variant="preview === true ? 'primary' : 'danger'" :disabled="m.busy" :loading="m.task === 'gc' && m.busy" @click="runGc">{{ preview === true ? 'Preview clean-up' : 'Clean up…' }}</Button>
        <span class="muted">{{ last('gc') }}</span>
        <div v-if="running('gc')" class="bar" role="status" data-testid="gc-progress">
          <span>{{ m.running!.phase || 'Starting…' }}</span>
          <ProgressBar label="Cleaning up" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
        </div>
        <div v-if="m.gc && !running('gc')" class="result" data-testid="gc-result" role="status">
          <strong class="head2">
            <CircleCheck v-if="!m.gc.report.errors.length" :size="16" aria-hidden="true" />
            <TriangleAlert v-else :size="16" aria-hidden="true" />
            {{ m.gc.preview ? 'Preview: this would free' : 'Freed' }} {{ formatBytes(m.gc.report.bytes_reclaimed + m.gc.report.chunk_bytes_reclaimed + m.gc.report.delta_bytes_reclaimed) }}
          </strong>
          <dl class="kv">
            <dt>Objects looked at</dt><dd class="mono">{{ n(m.gc.report.objects_scanned) }}</dd>
            <dt>Still needed</dt><dd class="mono">{{ n(m.gc.report.reachable_objects) }}</dd>
            <dt>Not needed</dt><dd class="mono">{{ n(m.gc.report.unreachable_objects) }}</dd>
            <dt>{{ m.gc.preview ? 'Would be removed' : 'Removed' }}</dt><dd class="mono">{{ n(m.gc.report.objects_deleted + m.gc.report.chunks_deleted + m.gc.report.deltas_deleted) }}</dd>
          </dl>
          <ul v-if="m.gc.report.errors.length" class="issues" aria-label="Clean-up problems">
            <li v-for="e in m.gc.report.errors" :key="e">{{ e }}</li>
          </ul>
          <Button v-if="m.gc.preview && (m.gc.report.objects_deleted || m.gc.report.unreachable_objects)" size="sm" :disabled="m.busy" @click="confirmOpen = true">Clean up now…</Button>
        </div>
      </section>

      <!-- Integrity check and verify share one result layout -->
      <section v-for="t in ([['fsck', 'Check integrity', 'Find missing or damaged data.'], ['verify', 'Verify repository', 'Confirm every object matches its hash.']] as const)" :key="t[0]" class="card" :aria-labelledby="`${t[0]}-h`" :data-task="t[0]">
        <h2 :id="`${t[0]}-h`">{{ t[1] }}</h2>
        <p class="muted">{{ t[2] }}</p>
        <Checkbox v-if="t[0] === 'fsck'" v-model="deep" label="Deep check (slow)" :disabled="m.busy" />
        <Checkbox v-else v-model="quick" label="Quick check" :disabled="m.busy" />
        <Button :disabled="m.busy" :loading="m.task === t[0] && m.busy" @click="t[0] === 'fsck' ? m.runFsck(deep === true) : m.runVerify(quick === true)">
          {{ t[0] === 'fsck' ? 'Run integrity check' : 'Verify' }}
        </Button>
        <span class="muted">{{ last(t[0]) }}</span>
        <div v-if="running(t[0])" class="bar" role="status" :data-testid="`${t[0]}-progress`">
          <span>{{ m.running!.phase || 'Starting…' }}</span>
          <ProgressBar :label="t[1]" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
        </div>
        <div v-if="m[t[0]] && !running(t[0])" class="result" :data-testid="`${t[0]}-result`" role="status">
          <strong class="head2" :class="m[t[0]]!.report.healthy ? 'ok' : 'bad'">
            <CircleCheck v-if="m[t[0]]!.report.healthy" :size="16" aria-hidden="true" />
            <TriangleAlert v-else :size="16" aria-hidden="true" />
            {{ m[t[0]]!.report.healthy ? 'No problems found' : 'Problems found' }}
          </strong>
          <dl class="kv">
            <dt>Objects checked</dt><dd class="mono">{{ n(m[t[0]]!.report.objects_checked) }}</dd>
            <dt>Refs checked</dt><dd class="mono">{{ n(m[t[0]]!.report.refs_checked) }}</dd>
            <dt>Damaged</dt><dd class="mono" :class="{ bad: m[t[0]]!.report.corrupted_objects }">{{ n(m[t[0]]!.report.corrupted_objects) }}</dd>
            <dt>Missing</dt><dd class="mono" :class="{ bad: m[t[0]]!.report.missing_objects }">{{ n(m[t[0]]!.report.missing_objects) }}</dd>
            <dt>Broken refs</dt><dd class="mono" :class="{ bad: m[t[0]]!.report.broken_refs }">{{ n(m[t[0]]!.report.broken_refs) }}</dd>
          </dl>
          <ul v-if="m[t[0]]!.report.issues.length" class="issues" :aria-label="`${t[1]} findings`">
            <li v-for="(i, k) in m[t[0]]!.report.issues.slice(0, 50)" :key="k"><span class="chip" :class="warn(i.severity) ? 'warn' : 'bad'">{{ warn(i.severity) ? 'Warning' : 'Error' }}</span> {{ i.message }}</li>
          </ul>
        </div>
      </section>
    </div>

    <AlertDialog
      v-model:open="confirmOpen"
      title="Clean up storage?"
      description="Data that no version needs is removed for good. Versions, branches and tags are not touched, but removed data cannot be brought back."
      confirm-label="Clean up"
      destructive
      @confirm="m.runGc(false, true)"
    />
  </section>
</template>

<style scoped>
.mt {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ttl {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
}
h1 {
  margin: 0;
  font-size: var(--fs-2xl);
}
h2 {
  margin: 0;
  font-size: var(--fs-lg);
}
.ttl p,
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.ttl p {
  font-size: var(--fs-md);
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: var(--space-4);
  align-items: start;
}
.card {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.bar {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
.result {
  display: grid;
  gap: var(--space-2);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border);
}
.head2 {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.head2.ok {
  color: var(--success);
}
.head2.bad {
  color: var(--danger);
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  margin: 0;
  font-size: var(--fs-sm);
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
}
.issues {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: var(--fs-sm);
  overflow-wrap: anywhere;
}
.kv dd.bad {
  color: var(--danger);
  font-weight: 600;
}
.chip {
  display: inline-block;
  padding: 1px var(--space-2);
  border-radius: var(--radius-pill);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.chip.bad {
  background: var(--danger-soft);
  color: var(--danger);
}
.chip.warn {
  background: var(--modified-soft);
  color: var(--modified);
}
</style>
