<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowDown, ArrowUp, Cloud, Download, Ellipsis, Plus, RefreshCw, Upload } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import DropdownMenu from '@/components/DropdownMenu.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
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

// Get latest / Send live in the hub header; this page keeps Fetch and the options for all three.
const verbs: { kind: TransferKind; label: string; hint: string; icon: typeof Download }[] = [
  { kind: 'fetch', label: 'Fetch', hint: 'Check what is new without changing your files', icon: RefreshCw },
  { kind: 'pull', label: 'Get latest', hint: 'Bring in your team’s new versions (pull)', icon: Download },
  { kind: 'push', label: 'Send', hint: 'Share your saved versions (push)', icon: Upload },
]
const fetchVerb = verbs[0]!

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
const fetchNow = ref(false)
const token = ref('')
const canAdd = computed(() => !!name.value && !!url.value && !nameError.value && !urlError.value && !sync.busy)
async function add() {
  if (!canAdd.value) return
  if (await sync.addRemote(name.value, url.value, { fetch: fetchNow.value, token: token.value })) {
    addOpen.value = false
    url.value = ''
    name.value = ''
    token.value = ''
    fetchNow.value = false
  }
}

// `removing` outlives the dialog: the confirm click closes it before our handler runs.
const removing = ref<string | null>(null)
const removeOpen = ref(false)
function askRemove(name: string) {
  removing.value = name
  removeOpen.value = true
}

// Per-remote actions ----------------------------------------------------
const REMOTE_MENU = [
  { id: 'details', label: 'Details…' },
  { id: 'rename', label: 'Rename…' },
  { id: 'url', label: 'Change address…' },
  { id: 'remove', label: 'Remove…', danger: true, separated: true },
]
const target = ref<{ name: string; url: string } | null>(null)
const detailOpen = ref(false)
const renameOpen = ref(false)
const urlOpen = ref(false)
const newName = ref('')
const newUrl = ref('')
const pushOnly = ref(false)
function pickAction(r: { name: string; url: string }, id: string) {
  target.value = r
  sync.actionError = null
  if (id === 'remove') askRemove(r.name)
  else if (id === 'details') {
    detailOpen.value = true
    void sync.showRemote(r.name)
  } else if (id === 'rename') {
    newName.value = r.name
    renameOpen.value = true
  } else {
    newUrl.value = r.url
    pushOnly.value = false
    urlOpen.value = true
  }
}
const renameError = computed(() =>
  newName.value && newName.value !== target.value?.name
    ? (refNameError(newName.value) ?? (sync.remotes.some((r) => r.name === newName.value) ? 'A remote with this name exists' : undefined))
    : undefined,
)
const canRename = computed(() => !!newName.value && newName.value !== target.value?.name && !renameError.value && !sync.busy)
async function rename() {
  if (canRename.value && target.value && (await sync.renameRemote(target.value.name, newName.value))) renameOpen.value = false
}
const newUrlError = computed(() => (newUrl.value && !/^https?:\/\/[^\s/]+/.test(newUrl.value) && !/^([a-zA-Z]:)?[\\/]/.test(newUrl.value) ? 'Use an http(s) address or a local folder path' : undefined))
const canSetUrl = computed(() => !!newUrl.value && !newUrlError.value && !sync.busy)
async function setUrl() {
  if (canSetUrl.value && target.value && (await sync.setRemoteUrl(target.value.name, newUrl.value, pushOnly.value))) urlOpen.value = false
}

// Transfer options ------------------------------------------------------
const optKind = ref<TransferKind>('push')
const OVERWRITE = [
  { value: 'none', label: 'Stop if the remote moved' },
  { value: 'lease', label: 'Overwrite safely' },
  { value: 'force', label: 'Overwrite anyway' },
]
const OVERWRITE_TEXT: Record<string, { text: string; flag?: string }> = {
  none: { text: 'Nothing is sent if someone else sent work since you last checked.' },
  lease: { text: 'Replaces the remote branch only if nobody sent new work since you last checked.', flag: '--force-with-lease' },
  force: { text: 'Replaces the remote branch even if teammates sent new work. Their versions are lost from the remote.', flag: '--force' },
}
const pushForm = sync.form.push
const confirmPushOpen = computed({ get: () => sync.confirmPushOpen, set: (v) => (sync.confirmPushOpen = v) })
const pushWarnings = computed(() => {
  const w: string[] = []
  const r = sync.remote ?? 'the remote'
  if (pushForm.delete) w.push(`Deletes ${pushForm.refspecs.trim() || 'the named branches'} on ${r} for everyone. Your own copies stay.`)
  if (!pushForm.delete && pushForm.overwrite === 'lease') w.push(`Replaces the branch on ${r} with yours. It stops if someone sent new work since your last fetch.`)
  if (!pushForm.delete && pushForm.overwrite === 'force') w.push(`Replaces the branch on ${r} with yours even if teammates sent new work. Their versions on that branch are lost from the remote.`)
  return w
})
const pushDeleteNeedsName = computed(() => sync.pushDeleteNeedsName)
function go(kind: TransferKind) {
  void sync.run(kind)
}
</script>

<template>
  <section class="sync" :aria-busy="sync.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Sync</h2>
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
            <span>{{ branch?.name ?? 'No branch' }}</span>
            <template v-if="up"> follows <span>{{ up.name }}</span></template>
            <template v-else> is not shared yet. Send it to share it.</template>
          </p>
        </div>
        <dl class="counts">
          <div>
            <dt>To send</dt>
            <dd class="num" :aria-label="up?.ahead == null ? 'Unknown until you fetch' : `${ahead} versions to send`"><ArrowUp :size="16" aria-hidden="true" />{{ up?.ahead == null ? '--' : ahead }}</dd>
          </div>
          <div>
            <dt>To get</dt>
            <dd class="num" :aria-label="up?.behind == null ? 'Unknown until you fetch' : `${behind} versions to get`"><ArrowDown :size="16" aria-hidden="true" />{{ up?.behind == null ? '--' : behind }}</dd>
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
          <h2 id="sync-act">Sync with <span v-if="sync.remotes.length === 1" class="rname">{{ sync.remote }}</span></h2>
          <Select v-if="sync.remotes.length > 1" v-model="picked" label="Remote" :options="remoteOptions" :disabled="sync.busy" />
        </div>
        <div class="verbs">
          <div class="verb">
            <Button variant="secondary" :disabled="sync.busy || !sync.remote" :loading="active?.kind === 'fetch'" @click="go('fetch')">
              <component :is="fetchVerb.icon" v-if="active?.kind !== 'fetch'" :size="16" aria-hidden="true" /> {{ fetchVerb.label }}
            </Button>
            <span class="hint">{{ fetchVerb.hint }}</span>
          </div>
        </div>

        <MoreOptions data-testid="sync-more">
          <p class="hint">These options apply to Fetch and to Get latest and Send at the top of the page.</p>
          <SegmentedControl v-model="optKind" label="Options for" :options="verbs.map((v) => ({ value: v.kind, label: v.label }))" />
          <div v-if="optKind === 'fetch'" class="opts">
            <TextField v-model="sync.form.fetch.branch" label="Only this branch" hint="Leave empty for the current branch." placeholder="main" mono :disabled="sync.form.fetch.all" />
            <OptionCheck v-model="sync.form.fetch.all" label="Every branch" flag="--all" />
            <OptionCheck v-model="sync.form.fetch.prune" label="Forget branches deleted on the remote" flag="--prune" />
          </div>
          <div v-else-if="optKind === 'pull'" class="opts">
            <TextField v-model="sync.form.pull.branch" label="Only this branch" hint="Leave empty to follow the branch you are on." placeholder="main" mono />
            <OptionCheck v-model="sync.form.pull.rebase" label="Replay my versions on top" hint="Instead of merging." flag="--rebase" />
            <OptionCheck v-model="sync.form.pull.dryRun" label="Preview only" hint="Check what would change; nothing is applied." flag="--dry-run" />
          </div>
          <div v-else class="opts">
            <TextField v-model="pushForm.refspecs" label="Branches to send" hint="Names separated by spaces, or local:remote. Leave empty for the current branch." placeholder="e.g. main dev:release" mono :disabled="pushForm.all" />
            <OptionCheck v-model="pushForm.all" label="Every branch" flag="--all" :disabled="pushForm.delete" />
            <OptionCheck v-model="pushForm.tags" label="Send tags too" flag="--tags" :disabled="pushForm.delete" />
            <OptionCheck v-model="pushForm.followTags" label="Send tags that point at what is sent" flag="--follow-tags" :disabled="pushForm.delete" />
            <OptionCheck v-model="pushForm.dryRun" label="Preview only" hint="Check what would be sent; nothing leaves this computer." flag="--dry-run" />
            <OptionCheck v-model="pushForm.setUpstream" label="Make the remote branch this branch’s upstream" flag="--set-upstream" />
            <OptionCheck v-model="pushForm.noTrack" label="Do not track the remote branch" flag="--no-track" :disabled="pushForm.setUpstream" />
            <OptionCheck v-model="pushForm.repair" label="Check the remote and re-send damaged data" flag="--repair" />
            <fieldset class="risk" data-testid="push-risk">
              <legend>If the remote has newer work</legend>
              <SegmentedControl v-model="pushForm.overwrite" label="If the remote has newer work" :options="OVERWRITE" :disabled="pushForm.delete" />
              <p class="rhint" data-testid="overwrite-hint">
                {{ pushForm.delete ? 'Not used while deleting branches.' : OVERWRITE_TEXT[pushForm.overwrite]?.text }}
                <code v-if="!pushForm.delete && OVERWRITE_TEXT[pushForm.overwrite]?.flag" class="mono flag">{{ OVERWRITE_TEXT[pushForm.overwrite]?.flag }}</code>
              </p>
            </fieldset>
            <fieldset class="risk danger-group" data-testid="push-delete">
              <legend>Remove branches</legend>
              <OptionCheck v-model="pushForm.delete" danger label="Delete these branches on the remote" hint="Name them above. This removes them for everyone." flag="--delete" />
              <p v-if="pushDeleteNeedsName" class="derr" role="alert">Name the branches to delete.</p>
            </fieldset>
          </div>
        </MoreOptions>

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
        <div class="arow">
          <h2 id="sync-remotes">Remotes</h2>
          <Button :disabled="sync.busy" @click="openAdd"><Plus :size="16" aria-hidden="true" /> Add remote</Button>
        </div>
        <ul>
          <li v-for="r in sync.remotes" :key="r.name" :data-remote="r.name">
            <Cloud :size="18" class="ci" aria-hidden="true" />
            <div class="rn">
              <strong>{{ r.name }}</strong>
              <span class="mono url" :title="r.url">{{ r.url }}</span>
            </div>
            <DropdownMenu :items="REMOTE_MENU" @select="(id) => pickAction(r, id)">
              <IconButton :label="`Actions for ${r.name}`" size="sm" :disabled="sync.busy"><Ellipsis :size="16" /></IconButton>
            </DropdownMenu>
          </li>
        </ul>
      </section>
    </template>

    <Dialog v-model:open="addOpen" title="Add remote" description="The address your team shares versions through.">
      <form id="add-remote" class="form" @submit.prevent="add">
        <TextField v-model="name" label="Name" placeholder="origin" :error="nameError" mono />
        <TextField v-model="url" label="Address" placeholder="https://mediagit.studio.example/hero-film" :error="urlError" mono />
        <MoreOptions data-testid="remote-more">
          <OptionCheck v-model="fetchNow" label="Fetch right after adding" flag="--fetch" />
          <TextField
            v-model="token"
            type="password"
            label="Access token"
            hint="Optional. Saved in this project’s settings file; to use your system keychain, sign in under Security instead." flag="--token-stdin"
            mono
          />
        </MoreOptions>
        <p v-if="sync.actionError && addOpen" class="derr" role="alert">{{ sync.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="addOpen = false">Cancel</Button>
        <Button type="submit" form="add-remote" variant="primary" :disabled="!canAdd" :loading="sync.busy">Add remote</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="renameOpen" :title="`Rename ${target?.name ?? ''}`" description="Branches that follow it keep working.">
      <form id="rename-remote" class="form" @submit.prevent="rename">
        <TextField v-model="newName" label="New name" :error="renameError" mono />
        <p v-if="sync.actionError && renameOpen" class="derr" role="alert">{{ sync.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="renameOpen = false">Cancel</Button>
        <Button type="submit" form="rename-remote" variant="primary" :disabled="!canRename" :loading="sync.busy">Rename</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="urlOpen" :title="`Change address of ${target?.name ?? ''}`" description="Where this remote lives.">
      <form id="seturl-remote" class="form" @submit.prevent="setUrl">
        <TextField v-model="newUrl" label="Address" :error="newUrlError" mono />
        <MoreOptions data-testid="url-more">
          <OptionCheck v-model="pushOnly" label="Only where versions are sent" hint="Fetching keeps using the old address." flag="--push" />
        </MoreOptions>
        <p v-if="sync.actionError && urlOpen" class="derr" role="alert">{{ sync.actionError.message }}</p>
      </form>
      <template #footer>
        <Button @click="urlOpen = false">Cancel</Button>
        <Button type="submit" form="seturl-remote" variant="primary" :disabled="!canSetUrl" :loading="sync.busy">Change address</Button>
      </template>
    </Dialog>

    <Dialog v-model:open="detailOpen" :title="target?.name ?? 'Remote'" description="What the server says about this remote.">
      <p v-if="sync.actionError && detailOpen" class="derr" role="alert">{{ sync.actionError.message }}</p>
      <p v-else-if="!sync.detail" role="status">Asking the server…</p>
      <dl v-else class="detail" data-testid="remote-detail">
        <dt>Fetch address</dt><dd class="mono">{{ sync.detail.fetch_url }}</dd>
        <dt>Send address</dt><dd class="mono">{{ sync.detail.push_url }}</dd>
        <dt>Reachable</dt><dd>{{ sync.detail.reachable ? 'Yes' : 'No' }}<template v-if="sync.detail.error"> · {{ sync.detail.error }}</template></dd>
        <dt>Default branch</dt><dd class="mono">{{ sync.detail.head_branch ?? 'unknown' }}</dd>
        <dt>Branches</dt><dd class="mono">{{ sync.detail.branches.join(', ') || 'none' }}</dd>
      </dl>
      <template #footer><Button @click="detailOpen = false">Close</Button></template>
    </Dialog>

    <AlertDialog
      v-model:open="confirmPushOpen"
      :title="pushForm.delete ? 'Delete on the remote?' : 'Overwrite on the remote?'"
      :description="pushWarnings.join(' ')"
      :confirm-label="pushForm.delete ? 'Delete branches' : 'Overwrite'"
      destructive
      @confirm="sync.run('push', true)"
    />

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
.pt {
  margin: 0;
  font-size: var(--fs-xl);
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
  font-size: var(--fs-lg);
  font-weight: 600;
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
.counts dd:not(.num) {
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
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}
.verbs > .hint {
  margin: 0;
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
.opts {
  display: grid;
  gap: var(--space-3);
  max-width: 560px;
}
.risk {
  display: grid;
  gap: var(--space-3);
  margin: var(--space-2) 0 0;
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
}
.risk legend {
  padding: 0 var(--space-2);
  font-size: var(--fs-sm);
  font-weight: 500;
}
.danger-group {
  border-color: var(--danger);
}
.danger-group legend {
  color: var(--danger);
}
.flag {
  white-space: nowrap;
}
.rhint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.detail {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-2) var(--space-4);
  margin: 0;
}
.detail dt {
  color: var(--text-muted);
}
.detail dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.derr {
  margin: 0;
  color: var(--danger);
  font-size: var(--fs-sm);
}
</style>
