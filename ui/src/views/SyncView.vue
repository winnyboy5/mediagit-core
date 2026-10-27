<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, Cloud, Download, RefreshCw, Trash2, Upload } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TextField from '@/components/TextField.vue'
import TransferStats from '@/components/TransferStats.vue'
import { refNameError } from '@/lib/inspect'
import { timeAgo } from '@/lib/persist'
import { etaOf, primaryBar, rateOf, useOpsStore } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useSyncStore, type TransferKind } from '@/stores/sync'

const sync = useSyncStore()
const repo = useRepoStore()
const ops = useOpsStore()
onMounted(() => void sync.refresh())

const firstLoad = computed(() => !sync.loaded && !sync.error)
const up = computed(() => repo.status?.branch.upstream ?? null)
const branch = computed(() => repo.status?.branch)
const ahead = computed(() => up.value?.ahead ?? 0)
const behind = computed(() => up.value?.behind ?? 0)
const remoteOptions = computed(() => sync.remotes.map((r) => ({ value: r.name, label: r.name })))
const picked = computed({
  get: () => sync.remote ?? undefined,
  set: (v) => (sync.chosen = v ?? null),
})

const verbs: { kind: TransferKind; label: string; hint: string; icon: typeof Download }[] = [
  { kind: 'fetch', label: 'Fetch', hint: 'Check what is new without changing your files', icon: RefreshCw },
  { kind: 'pull', label: 'Get latest', hint: 'Bring in your team’s new versions (pull)', icon: Download },
  { kind: 'push', label: 'Send', hint: 'Share your saved versions (push)', icon: Upload },
]

// The transfer in flight, with every bar that is still moving.
const active = computed(() => sync.active)
const bars = computed(() => (active.value ? [...active.value.bars.values()].filter((b) => !b.done) : []))
const lead = computed(() => (active.value ? primaryBar(active.value) : undefined))
const whenLast = computed(() => (sync.last ? timeAgo(sync.last, ops.now) : null))

// Add remote ------------------------------------------------------------
const addOpen = ref(false)
const name = ref('origin')
const url = ref('')
const nameError = computed(() => (name.value ? (refNameError(name.value) ?? (sync.remotes.some((r) => r.name === name.value) ? 'A remote with this name exists' : undefined)) : undefined))
const urlError = computed(() =>
  url.value && !/^https?:\/\/[^\s/]+/.test(url.value) && !/^([a-zA-Z]:)?[\\/]/.test(url.value) ? 'Use an http(s) address or a local folder path' : undefined,
)
function openAdd() {
  sync.actionError = null
  addOpen.value = true
}
const canAdd = computed(() => !!name.value && !!url.value && !nameError.value && !urlError.value && !sync.busy)
async function add() {
  if (!canAdd.value) return
  if (await sync.addRemote(name.value, url.value)) {
    addOpen.value = false
    url.value = ''
    name.value = ''
  }
}

// `removing` outlives the dialog: the confirm click closes it before our handler runs.
const removing = ref<string | null>(null)
const removeOpen = ref(false)
function askRemove(name: string) {
  removing.value = name
  removeOpen.value = true
}
</script>

<template>
  <section class="sync" :aria-busy="sync.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Sync</h1>
        <p>share work with your team</p>
      </div>
      <div class="tools">
        <IconButton label="Refresh remotes" variant="secondary" :disabled="sync.loading" @click="sync.refresh()"><RefreshCw :size="18" /></IconButton>
        <Button variant="primary" :disabled="sync.busy" @click="openAdd">Add remote</Button>
      </div>
    </header>

    <div v-if="sync.actionError" class="banner" role="alert">
      <code class="mono">{{ sync.actionError.code }}</code>
      <span>{{ sync.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="sync.actionError = null">Dismiss</Button>
    </div>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="4" /></div>
    <div v-else-if="sync.error && !sync.loaded" class="pad">
      <ErrorState :code="sync.error.code" :message="sync.error.message" @retry="sync.refresh()" />
    </div>
    <div v-else-if="!sync.remotes.length" class="pad">
      <EmptyState title="No remote yet" description="Add the address of your studio’s MediaGit server or bucket to share versions.">
        <template #icon><Cloud :size="24" /></template>
        <Button variant="primary" @click="openAdd">Add remote</Button>
      </EmptyState>
    </div>

    <template v-else>
      <section class="card" aria-labelledby="sync-state" data-testid="sync-state">
        <div class="where">
          <h2 id="sync-state">Where you stand</h2>
          <p class="branchline">
            <span class="mono">{{ branch?.name ?? 'No branch' }}</span>
            <template v-if="up"> follows <span class="mono">{{ up.name }}</span></template>
            <template v-else> is not shared yet. Send it to share it.</template>
          </p>
        </div>
        <dl class="counts">
          <div>
            <dt>To send</dt>
            <dd class="mono" :aria-label="up?.ahead == null ? 'Unknown until you fetch' : `${ahead} versions to send`"><ArrowUp :size="16" aria-hidden="true" />{{ up?.ahead == null ? '--' : ahead }}</dd>
          </div>
          <div>
            <dt>To get</dt>
            <dd class="mono" :aria-label="up?.behind == null ? 'Unknown until you fetch' : `${behind} versions to get`"><ArrowDown :size="16" aria-hidden="true" />{{ up?.behind == null ? '--' : behind }}</dd>
          </div>
          <div>
            <dt>Last synced</dt>
            <dd>{{ whenLast ?? 'Not yet from this app' }}</dd>
          </div>
        </dl>
        <p v-if="up && (up.ahead == null || up.behind == null)" class="note">Fetch to find out how far ahead or behind you are.</p>
      </section>

      <section class="actions" aria-labelledby="sync-act">
        <div class="arow">
          <h2 id="sync-act">Sync with <span v-if="sync.remotes.length === 1" class="mono rname">{{ sync.remote }}</span></h2>
          <Select v-if="sync.remotes.length > 1" v-model="picked" label="Remote" :options="remoteOptions" :disabled="sync.busy" />
        </div>
        <div class="verbs">
          <div v-for="v in verbs" :key="v.kind" class="verb">
            <Button
              :variant="(v.kind === 'push' && ahead > 0) || (v.kind === 'pull' && behind > 0 && ahead === 0) ? 'primary' : 'secondary'"
              :disabled="sync.busy || !sync.remote"
              :loading="active?.kind === v.kind"
              @click="sync.run(v.kind)"
            >
              <component :is="v.icon" v-if="active?.kind !== v.kind" :size="16" aria-hidden="true" /> {{ v.label }}
            </Button>
            <span class="hint">{{ v.hint }}</span>
          </div>
        </div>

        <div v-if="active" class="run" data-testid="transfer-progress" role="status" aria-live="polite">
          <div class="rhead">
            <strong>{{ active.label }}</strong>
            <span class="phase">{{ active.phase || 'Connecting…' }}</span>
            <Button v-if="active.cancel" size="sm" variant="danger" @click="ops.cancel(active.id)">Cancel</Button>
          </div>
          <ul v-if="bars.length">
            <li v-for="b in bars" :key="b.id">
              <span class="bl">{{ b.msg || 'Working' }}</span>
              <ProgressBar :label="b.msg || active.label" :value="b.len != null ? b.pos : null" :max="b.len ?? 100" />
              <TransferStats v-if="b.unit === 'bytes'" :done="b.pos" :total="b.len" :rate="rateOf(b)" :eta="etaOf(b)" />
              <span v-else class="mono">{{ b.pos }}<template v-if="b.len != null"> / {{ b.len }}</template></span>
            </li>
          </ul>
          <ProgressBar v-else :label="active.label" :value="lead?.len != null ? lead.pos : null" />
        </div>
      </section>

      <section class="remotes" aria-labelledby="sync-remotes">
        <h2 id="sync-remotes">Remotes</h2>
        <ul>
          <li v-for="r in sync.remotes" :key="r.name" :data-remote="r.name">
            <Cloud :size="18" class="ci" aria-hidden="true" />
            <div class="rn">
              <strong class="mono">{{ r.name }}</strong>
              <span class="mono url" :title="r.url">{{ r.url }}</span>
            </div>
            <IconButton :label="`Remove ${r.name}`" size="sm" :disabled="sync.busy" @click="askRemove(r.name)"><Trash2 :size="16" /></IconButton>
          </li>
        </ul>
      </section>
    </template>

    <Dialog v-model:open="addOpen" title="Add remote" description="The address your team shares versions through.">
      <form id="add-remote" class="form" @submit.prevent="add">
        <TextField v-model="name" label="Name" placeholder="origin" :error="nameError" mono />
        <TextField v-model="url" label="Address" placeholder="https://mediagit.studio.example/hero-film" :error="urlError" mono />
        <p v-if="sync.actionError && addOpen" class="derr" role="alert">{{ sync.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="addOpen = false">Cancel</Button>
        <Button type="submit" form="add-remote" variant="primary" :disabled="!canAdd" :loading="sync.busy">Add remote</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="removeOpen"
      :title="`Remove ${removing ?? ''}?`"
      description="This project forgets the address. Nothing is deleted on the server and your versions stay here."
      confirm-label="Remove remote"
      destructive
      @confirm="removing && sync.removeRemote(removing)"
    />
  </section>
</template>

<style scoped>
.sync {
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
  gap: var(--space-4);
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
.ttl p {
  margin: 0;
  color: var(--text-muted);
}
.tools {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
h2 {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
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
.card,
.actions,
.remotes {
  display: grid;
  gap: var(--space-4);
  padding: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.branchline {
  margin: var(--space-2) 0 0;
}
.counts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-8);
  margin: 0;
}
.counts dt {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.counts dd {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  margin: var(--space-1) 0 0;
  font-size: var(--fs-xl);
  font-weight: 600;
}
.counts dd:not(.mono) {
  font-size: var(--fs-md);
  font-weight: 500;
}
.note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.arow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}
.rname {
  color: var(--text);
  text-transform: none;
  letter-spacing: 0;
}
.verbs {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--space-4);
}
.verb {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}
.hint {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.run {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.rhead {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.phase {
  flex: 1;
  color: var(--text-muted);
}
.run ul {
  display: grid;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  list-style: none;
}
.run li {
  display: grid;
  gap: var(--space-1);
}
.bl {
  font-size: var(--fs-sm);
}
.remotes ul {
  display: grid;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.remotes li {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: var(--row-h);
}
.ci {
  flex: none;
  color: var(--text-muted);
}
.rn {
  display: grid;
  flex: 1;
  min-width: 0;
}
.url {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pad {
  overflow-y: auto;
}
.form {
  display: grid;
  gap: var(--space-3);
}
.derr {
  margin: 0;
  color: var(--danger);
  font-size: var(--fs-sm);
}
@media (max-width: 1100px) {
  .verbs {
    grid-template-columns: 1fr;
  }
}
</style>
