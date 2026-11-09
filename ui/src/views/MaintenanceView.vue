<script setup lang="ts">
import { computed, ref } from 'vue'
import { CircleCheck, TriangleAlert } from 'lucide-vue-next'
import { formatBytes } from '@/components/format'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import TextField from '@/components/TextField.vue'
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

// More options (each is a CLI flag, shown as secondary text).
const noPrune = ref(false)
const auto = ref(false)
const repack = ref(false)
const gcOptions = () => ({ noPrune: noPrune.value === true, auto: auto.value === true, repack: repack.value === true })

const all = ref(false)
const lostFound = ref(false)
const noDangling = ref(false)
const repair = ref(false)
const repairPreview = ref(true)
const repairOpen = ref(false)
const fsckOptions = (confirmed: boolean) => ({
  deep: deep.value === true,
  all: all.value === true,
  lostFound: lostFound.value === true,
  noDangling: noDangling.value === true,
  repair: repair.value === true,
  dryRun: repair.value === true && repairPreview.value === true,
  confirmed,
})
function runFsck() {
  // A real repair changes the repository: say what can be lost first.
  if (repair.value === true && repairPreview.value !== true) repairOpen.value = true
  else void m.runFsck(fsckOptions(false))
}

const vCommit = ref('')
const vStart = ref('')
const vEnd = ref('')
const checksums = ref(false)
const detailed = ref(false)
const rangeClash = computed(() => (vCommit.value.trim() && (vStart.value.trim() || vEnd.value.trim()) ? 'Choose one version or a range, not both' : undefined))
const verifyOptions = () => ({
  quick: quick.value === true,
  commit: vCommit.value.trim() || null,
  start: vStart.value.trim() || null,
  end: vEnd.value.trim() || null,
  checksums: checksums.value === true,
  detailed: detailed.value === true,
})

function runGc() {
  if (preview.value === true) void m.runGc(true, false, gcOptions())
  else confirmOpen.value = true
}
</script>

<template>
  <section class="mt" :aria-busy="m.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Maintenance</h2>
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
        <MoreOptions>
          <OptionCheck v-model="noPrune" label="Keep data nothing refers to" flag="--no-prune" :disabled="m.busy" />
          <OptionCheck v-model="auto" label="Only if it is worth doing" flag="--auto" :disabled="m.busy" />
          <OptionCheck v-model="repack" label="Pack loose data together" flag="--repack" :disabled="m.busy" />
        </MoreOptions>
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
        <MoreOptions v-if="t[0] === 'fsck'">
          <OptionCheck v-model="all" label="List every object checked" flag="--all" :disabled="m.busy" />
          <OptionCheck v-model="lostFound" label="List lost and unused objects" flag="--lost-found" :disabled="m.busy" />
          <OptionCheck v-model="noDangling" label="Skip the unused-objects check" flag="--no-dangling" :disabled="m.busy" />
          <OptionCheck v-model="repair" label="Try to repair what it finds" flag="--repair" :disabled="m.busy" />
          <OptionCheck v-if="repair === true" v-model="repairPreview" label="Only show what a repair would change" flag="--dry-run" :disabled="m.busy" />
        </MoreOptions>
        <MoreOptions v-else>
          <TextField v-model="vCommit" label="Check only this version" placeholder="e.g. main, HEAD or a version id" hint="Leave empty to check everything." mono :error="rangeClash" />
          <TextField v-model="vStart" label="From version" hint="Check history from here." flag="--start" mono />
          <TextField v-model="vEnd" label="Up to version" hint="Check history up to here." flag="--end" mono />
          <OptionCheck v-model="checksums" label="Check every checksum" flag="--checksums" :disabled="m.busy" />
          <OptionCheck v-model="detailed" label="Describe each problem in detail" flag="--detailed" :disabled="m.busy" />
        </MoreOptions>
        <Button
          :variant="t[0] === 'fsck' && repair === true && repairPreview !== true ? 'danger' : 'secondary'"
          :disabled="m.busy || (t[0] === 'verify' && !!rangeClash)"
          :loading="m.task === t[0] && m.busy"
          @click="t[0] === 'fsck' ? runFsck() : m.runVerify(verifyOptions())"
        >
          {{ t[0] === 'fsck' ? (repair === true ? (repairPreview === true ? 'Preview repair' : 'Repair…') : 'Run integrity check') : 'Verify' }}
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
            <template v-if="t[0] === 'fsck' && m.fsck?.repair">
              <dt>{{ m.fsck.repair === 'preview' ? 'Would repair' : 'Repaired' }}</dt><dd class="mono">{{ n(m.fsck.report.repaired ?? 0) }}</dd>
            </template>
          </dl>
          <ul v-if="m[t[0]]!.report.issues.length" class="issues" :aria-label="`${t[1]} findings`">
            <li v-for="(i, k) in m[t[0]]!.report.issues.slice(0, 50)" :key="k"><span class="chip" :class="warn(i.severity) ? 'warn' : 'bad'">{{ warn(i.severity) ? 'Warning' : 'Error' }}</span> {{ i.message }}</li>
          </ul>
        </div>
        <div v-if="t[0] === 'verify' && m.verifyRange && !running('verify')" class="result" data-testid="verify-range-result" role="status">
          <strong class="head2" :class="m.verifyRange.report.errors ? 'bad' : 'ok'">
            <CircleCheck v-if="!m.verifyRange.report.errors" :size="16" aria-hidden="true" />
            <TriangleAlert v-else :size="16" aria-hidden="true" />
            {{ m.verifyRange.report.errors ? 'Problems found' : 'No problems found' }}
          </strong>
          <dl class="kv">
            <dt>Versions looked at</dt><dd class="mono">{{ n(m.verifyRange.report.commits) }}</dd>
            <dt>Verified</dt><dd class="mono">{{ n(m.verifyRange.report.verified) }}</dd>
            <dt>Problems</dt><dd class="mono" :class="{ bad: m.verifyRange.report.errors }">{{ n(m.verifyRange.report.errors) }}</dd>
          </dl>
          <ul v-if="m.verifyRange.report.problems.length" class="issues" aria-label="Verify findings">
            <li v-for="p in m.verifyRange.report.problems.slice(0, 50)" :key="p">{{ p }}</li>
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
      @confirm="m.runGc(false, true, gcOptions())"
    />
    <AlertDialog
      v-model:open="repairOpen"
      title="Repair the repository?"
      description="Repair rewrites or removes damaged and unused data it finds. Data it removes cannot be brought back. Run the preview first to see what it would change."
      confirm-label="Repair"
      destructive
      @confirm="m.runFsck(fsckOptions(true))"
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
.pt {
  margin: 0;
  font-size: var(--fs-xl);
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
