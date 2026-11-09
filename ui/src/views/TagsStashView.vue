<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Archive, Ellipsis, Tag } from 'lucide-vue-next'
import DropdownMenu from '@/components/DropdownMenu.vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import Dialog from '@/components/Dialog.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TagPill from '@/components/TagPill.vue'
import TextField from '@/components/TextField.vue'
import { refNameError } from '@/lib/inspect'
import { shortOid } from '@/lib/history'
import { timeAgo } from '@/lib/persist'
import { primaryBar } from '@/stores/ops'
import { useRepoStore } from '@/stores/repo'
import { useTagsStashStore } from '@/stores/tagsStash'

// Which half this page shows: Tags lives in History, Stash in Branches.
const props = defineProps<{ only?: 'tags' | 'stash' }>()
const ts = useTagsStashStore()
const repo = useRepoStore()
// The shell already loads these for the section counts; do not ask twice while that is in flight.
onMounted(() => !ts.loading && void ts.refresh())

const message = ref('')
const includeUntracked = ref<boolean | 'indeterminate'>(false)

const s = computed(() => repo.status?.summary)
const tracked = computed(() => (s.value ? s.value.staged + s.value.modified + s.value.deleted : 0))
const bar = computed(() => (ts.running ? primaryBar(ts.running) : undefined))

// "Shelve selected files": only these paths go into the stash (`stash push -- paths`).
const SHOWN = 100
const picked = ref<Set<string>>(new Set())
const changed = computed(() => {
  const st = repo.status
  if (!st) return []
  const all = [
    ...st.staged.map((f) => f.path),
    ...st.modified,
    ...st.deleted,
    ...(includeUntracked.value === true ? st.untracked : []),
  ]
  return [...new Set(all)]
})
const pickedPaths = computed(() => changed.value.filter((p) => picked.value.has(p)))
const stashable = computed(() =>
  pickedPaths.value.length ? true : tracked.value > 0 || (includeUntracked.value === true && (s.value?.untracked ?? 0) > 0),
)
const saveLabel = computed(() =>
  pickedPaths.value.length ? `Set aside ${pickedPaths.value.length} selected file${pickedPaths.value.length === 1 ? '' : 's'}` : 'Stash my changes',
)
function togglePath(p: string, on: boolean | 'indeterminate') {
  const next = new Set(picked.value)
  if (on === true) next.add(p)
  else next.delete(p)
  picked.value = next
}

async function save() {
  if (!stashable.value || ts.busy) return
  if (await ts.save(message.value, includeUntracked.value === true, pickedPaths.value)) {
    message.value = ''
    picked.value = new Set()
  }
}

// Tags ------------------------------------------------------------------
const tagName = ref('')
const tagNote = ref('')
const tagCommit = ref('')
const tagger = ref('')
const taggerEmail = ref('')
const replace = ref(false)
const replaceOpen = ref(false)
const taken = computed(() => ts.tags.some((t) => t.name === tagName.value))
const tagError = computed(() =>
  tagName.value ? (refNameError(tagName.value) ?? (taken.value && replace.value !== true ? 'A tag with this name exists' : undefined)) : undefined,
)
const commitError = computed(() => (tagCommit.value.trim() && !/^[0-9a-f]{7,64}$/i.test(tagCommit.value.trim()) ? 'Use a version id from History (7 or more letters and digits)' : undefined))
const canTag = computed(() => !!tagName.value && !tagError.value && !commitError.value && !ts.busy && (!!repo.status?.branch.name || !!tagCommit.value.trim()))
const hasNote = computed(() => !!tagNote.value.trim())
async function doTag(confirmed: boolean) {
  const replacing = replace.value === true
  const ok = await ts.createTag(tagName.value, {
    commit: tagCommit.value.trim() || null,
    message: tagNote.value,
    tagger: hasNote.value ? tagger.value.trim() || null : null,
    email: hasNote.value ? taggerEmail.value.trim() || null : null,
    force: replacing,
    confirmed: replacing && confirmed,
  })
  if (ok) {
    tagName.value = tagNote.value = tagCommit.value = tagger.value = taggerEmail.value = ''
    replace.value = false
  }
}
function createTag() {
  if (!canTag.value) return
  // Replacing an existing tag loses where it pointed: say so first.
  if (replace.value === true && taken.value) replaceOpen.value = true
  else void doTag(false)
}
// The tag filter (`tag list PATTERN`) applies after a short pause in typing.
let filterTimer: ReturnType<typeof setTimeout> | undefined
function onFilter() {
  clearTimeout(filterTimer)
  filterTimer = setTimeout(() => void ts.refresh(), 250)
}
onBeforeUnmount(() => clearTimeout(filterTimer))
const detailOpen = computed({ get: () => ts.detail !== null, set: (v) => !v && ts.closeTag() })
const detailDate = computed(() => {
  const d = ts.detail?.show?.date
  return d && !Number.isNaN(Date.parse(d)) ? new Date(d).toLocaleString() : (d ?? '')
})

// Apply or pop with options (`--index`, `--force`).
const restoring = ref<number | null>(null)
const restoreOpen = ref(false)
const restoreIndex = ref(false)
const restoreForce = ref(false)
const restoreInfo = ref<{ oid: string; message: string | null; branch: string | null; timestamp: string } | null>(null)
async function askRestore(i: number) {
  restoring.value = i
  restoreIndex.value = restoreForce.value = false
  restoreInfo.value = null
  restoreOpen.value = true
  restoreInfo.value = (await ts.showStash(i)) as typeof restoreInfo.value
}
function restore(remove: boolean) {
  if (restoring.value === null) return
  const opts = { index: restoreIndex.value === true, force: restoreForce.value === true }
  restoreOpen.value = false
  void (remove ? ts.pop(restoring.value, opts) : ts.apply(restoring.value, opts))
}
const TAG_MENU = [
  { id: 'details', label: 'Details…' },
  { id: 'delete', label: 'Delete tag…', danger: true, separated: true },
]
const tagDeleting = ref<string | null>(null)
const tagDeleteOpen = ref(false)
function askDeleteTag(name: string) {
  tagDeleting.value = name
  tagDeleteOpen.value = true
}

// `dropping` outlives the dialog: the confirm click closes it before our handler runs.
const dropping = ref<number | null>(null)
const dropOpen = ref(false)
const clearOpen = ref(false)
function askDrop(i: number) {
  dropping.value = i
  dropOpen.value = true
}
const when = (iso: string) => (Number.isNaN(Date.parse(iso)) ? '' : timeAgo(Date.parse(iso)))
</script>

<template>
  <section class="ts" :aria-busy="ts.busy || undefined">
    <h2 v-if="!props.only" class="pt">Tags &amp; Stash</h2>

    <div v-if="ts.actionError" class="banner" role="alert">
      <code class="mono">{{ ts.actionError.code }}</code>
      <span>{{ ts.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="ts.actionError = null">Dismiss</Button>
    </div>

    <div class="grid">
      <section v-if="props.only !== 'stash'" class="card" :class="{ bare: props.only }" aria-labelledby="tags-h">
        <h2 id="tags-h" class="sr-only">Tags</h2>
        <TextField v-if="ts.tags.length || ts.filter" v-model="ts.filter" label="Show only tags matching" placeholder="e.g. v1.*" hint="Use * for any letters, like v1.*" mono @update:model-value="onFilter" />
        <div v-if="!ts.tagsLoaded && !ts.tagsError"><SkeletonRows :count="3" /></div>
        <ErrorState v-else-if="ts.tagsError" :code="ts.tagsError.code" :message="ts.tagsError.message" @retry="ts.refresh()" />
        <EmptyState v-else-if="!ts.tags.length && ts.filter" title="No tag matches" description="Change the filter, or clear it to see every tag." />
        <EmptyState v-else-if="!ts.tags.length" title="No tags yet" description="A tag names a version, like v1.0 delivery. Tags made elsewhere show up here.">
          <template #icon><Tag :size="24" /></template>
        </EmptyState>
        <ul v-else class="rows" aria-label="Tags">
          <li v-for="t in ts.tags" :key="t.name" class="row" :data-tag="t.name">
            <TagPill :name="t.name" />
            <span class="nm">{{ t.message || (t.annotated ? '' : 'Plain tag') }}</span>
            <span class="oid mono">{{ shortOid(t.oid) }}</span>
            <DropdownMenu :items="TAG_MENU" @select="(id) => (id === 'details' ? ts.showTag(t.name) : askDeleteTag(t.name))">
              <IconButton :label="`Actions for tag ${t.name}`" size="sm" :disabled="ts.busy"><Ellipsis :size="16" /></IconButton>
            </DropdownMenu>
          </li>
        </ul>
        <MoreOptions label="New tag" data-testid="new-tag">
        <form class="save" @submit.prevent="createTag">
          <TextField v-model="tagName" label="Tag name" placeholder="e.g. v1.0" :error="tagError ?? undefined" mono />
          <TextField v-model="tagNote" label="Note (optional)" placeholder="e.g. Final delivery" />
          <MoreOptions>
            <TextField v-model="tagCommit" label="Version to tag" placeholder="e.g. a version id from History" hint="Leave empty to tag the latest version." :error="commitError" mono />
            <TextField v-model="tagger" label="Tagger name" :disabled="!hasNote" :hint="hasNote ? 'Leave empty to use your own name.' : 'Add a note first to set these.'" flag="--tagger" />
            <TextField v-model="taggerEmail" label="Tagger email" type="email" :disabled="!hasNote" :hint="hasNote ? 'Leave empty to use your own email.' : 'Add a note first to set these.'" flag="--email" />
            <OptionCheck v-model="replace" label="Replace the tag if the name is taken" flag="--force" danger />
          </MoreOptions>
          <Button type="submit" class="end" :variant="replace === true && taken ? 'danger' : 'secondary'" :disabled="!canTag" :loading="ts.busy && !ts.running">
            {{ replace === true && taken ? 'Replace tag…' : tagCommit.trim() ? 'Tag this version' : 'Tag the latest version' }}
          </Button>
        </form>
        </MoreOptions>
      </section>

      <section v-if="props.only !== 'tags'" class="card" aria-labelledby="stash-h">
        <div class="card-head">
          <h2 id="stash-h">Stash</h2>
          <Button v-if="ts.stashes.length" size="sm" variant="secondary" class="dtxt" :disabled="ts.busy" @click="clearOpen = true">Remove all</Button>
        </div>
        <form class="save" @submit.prevent="save">
          <TextField v-model="message" label="Message (optional)" placeholder="WIP lighting" />
          <Checkbox v-model="includeUntracked" label="Include new files" />
          <MoreOptions label="Choose which files to set aside">
            <p class="hint">Tick files to set aside only those. Everything else stays as it is. With nothing ticked, all changes are stashed.</p>
            <ul v-if="changed.length" class="picks" aria-label="Changed files" data-testid="stash-files">
              <li v-for="p in changed.slice(0, SHOWN)" :key="p">
                <Checkbox :model-value="picked.has(p)" :label="p" @update:model-value="togglePath(p, $event)" />
              </li>
            </ul>
            <p v-else class="hint">No changed files.</p>
            <p v-if="changed.length > SHOWN" class="hint">Showing the first {{ SHOWN }} of {{ changed.length }} files.</p>
          </MoreOptions>
          <Button type="submit" variant="secondary" class="end" :disabled="!stashable || ts.busy" :loading="ts.busy && !ts.running">{{ saveLabel }}</Button>
          <p v-if="!stashable" class="hint">You have no unsaved changes to set aside.</p>
        </form>

        <div v-if="ts.running" class="busy" role="status" data-testid="stash-progress">
          <span class="mono">{{ ts.running.label }}<template v-if="bar && bar.len != null">: {{ bar.pos }} / {{ bar.len }} files</template></span>
          <ProgressBar :label="ts.running.label" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
        </div>

        <div v-if="!ts.stashLoaded && !ts.stashError"><SkeletonRows :count="3" /></div>
        <ErrorState v-else-if="ts.stashError" :code="ts.stashError.code" :message="ts.stashError.message" @retry="ts.refresh()" />
        <EmptyState v-else-if="!ts.stashes.length" title="Nothing set aside" description="Stash parks your unsaved changes so you can switch tasks, then bring them back.">
          <template #icon><Archive :size="24" /></template>
        </EmptyState>
        <ul v-else class="rows" aria-label="Stashed work">
          <li v-for="e in ts.stashes" :key="e.oid + e.index" class="row tall" :data-stash="e.index">
            <span class="oid mono">#{{ e.index }}</span>
            <div class="nm">
              <strong>{{ e.message || '(no message)' }}</strong>
              <span class="muted">{{ [e.branch, when(e.timestamp)].filter(Boolean).join(' · ') }}</span>
            </div>
            <div class="acts">
              <Button size="sm" :disabled="ts.busy" :aria-label="`Apply stash ${e.index}`" @click="ts.apply(e.index)">Apply</Button>
              <Button size="sm" :disabled="ts.busy" :aria-label="`Pop stash ${e.index}`" @click="ts.pop(e.index)">Pop</Button>
              <Button size="sm" variant="ghost" :disabled="ts.busy" :aria-label="`Options for stash ${e.index}`" @click="askRestore(e.index)">Options…</Button>
              <Button size="sm" variant="secondary" class="dtxt" :disabled="ts.busy" :aria-label="`Drop stash ${e.index}`" @click="askDrop(e.index)">Drop</Button>
            </div>
          </li>
        </ul>
        <p v-if="ts.stashes.length" class="hint">Apply keeps the entry. Pop applies it and removes it.</p>
      </section>
    </div>

    <Dialog v-model:open="detailOpen" :title="`Tag ${ts.detail?.name ?? ''}`" description="What this tag records.">
      <div v-if="ts.detail" data-testid="tag-detail">
        <p v-if="ts.detail.error" class="err" role="alert">{{ ts.detail.error.message }}</p>
        <SkeletonRows v-else-if="ts.detail.loading && !ts.detail.show" :count="3" />
        <dl v-else-if="ts.detail.show" class="kv">
          <dt>Kind</dt><dd>{{ ts.detail.show.annotated ? 'Annotated tag' : 'Plain tag' }}</dd>
          <dt>Points at</dt><dd class="mono">{{ shortOid(ts.detail.show.oid) }}</dd>
          <template v-if="ts.detail.show.message"><dt>Note</dt><dd>{{ ts.detail.show.message }}</dd></template>
          <template v-if="ts.detail.show.tagger_name"><dt>Tagged by</dt><dd>{{ ts.detail.show.tagger_name }}<template v-if="ts.detail.show.tagger_email"> ({{ ts.detail.show.tagger_email }})</template></dd></template>
          <template v-if="detailDate"><dt>Tagged on</dt><dd>{{ detailDate }}</dd></template>
          <dt>Signature</dt><dd>{{ ts.detail.show.signed ? 'Signed' : 'Not signed' }}</dd>
        </dl>
        <div v-if="ts.detail.show" class="verify">
          <Button size="sm" :disabled="ts.detail.loading" @click="ts.verifyTag()">Check signature</Button>
          <p v-if="ts.detail.verifyError" class="err" role="alert">{{ ts.detail.verifyError.message }}</p>
          <p v-else-if="ts.detail.verify && !ts.detail.verify.signed" class="hint" role="status">This tag is not signed, so there is nothing to check.</p>
          <p v-else-if="ts.detail.verify" class="hint" role="status">
            Signature is valid: the tag is intact. Signer key <span class="mono">{{ ts.detail.verify.fingerprint }}</span>. Compare it with the signer's key another way to know who signed it.
          </p>
        </div>
      </div>
      <template #footer><Button @click="ts.closeTag()">Close</Button></template>
    </Dialog>

    <AlertDialog
      v-model:open="replaceOpen"
      :title="`Replace tag ${tagName}?`"
      :description="`The name ${tagName} moves to the new version. The old tag is lost, and anyone using it gets the new version.`"
      confirm-label="Replace tag"
      destructive
      @confirm="doTag(true)"
    />

    <Dialog v-model:open="restoreOpen" :title="`Stash ${restoring ?? ''}`" description="Bring these changes back, with options.">
      <dl v-if="restoreInfo" class="kv" data-testid="stash-detail">
        <dt>Note</dt><dd>{{ restoreInfo.message || '(no message)' }}</dd>
        <dt>Branch</dt><dd>{{ restoreInfo.branch || 'unknown' }}</dd>
        <dt>Set aside</dt><dd>{{ when(restoreInfo.timestamp) }}</dd>
      </dl>
      <div class="opts">
        <OptionCheck v-model="restoreIndex" label="Also restore what was staged for saving" flag="--index" />
        <OptionCheck v-model="restoreForce" label="Overwrite my current changes to these files" flag="--force" danger />
        <p v-if="restoreForce === true" class="warn" role="note">Files you changed since are replaced by the stashed version. Your newer edits to them are lost.</p>
      </div>
      <template #footer>
        <Button @click="restoreOpen = false">Cancel</Button>
        <Button :variant="restoreForce === true ? 'danger' : 'secondary'" @click="restore(false)">{{ restoreForce === true ? 'Overwrite and apply' : 'Apply' }}</Button>
        <Button :variant="restoreForce === true ? 'danger' : 'primary'" @click="restore(true)">{{ restoreForce === true ? 'Overwrite, apply and remove' : 'Apply and remove' }}</Button>
      </template>
    </Dialog>

    <AlertDialog
      v-model:open="tagDeleteOpen"
      :title="`Delete tag ${tagDeleting ?? ''}?`"
      description="Only the name is removed. The version it pointed at stays in History."
      confirm-label="Delete tag"
      destructive
      @confirm="tagDeleting && ts.deleteTag(tagDeleting)"
    />
    <AlertDialog
      v-model:open="dropOpen"
      :title="`Drop stash ${dropping ?? ''}?`"
      description="The parked changes are removed for good. You cannot get them back."
      confirm-label="Drop stash"
      destructive
      @confirm="dropping !== null && ts.drop(dropping)"
    />
    <AlertDialog
      v-model:open="clearOpen"
      title="Remove every stash?"
      description="All parked changes are removed for good. You cannot get them back."
      confirm-label="Remove all"
      destructive
      @confirm="ts.clear()"
    />
  </section>
</template>

<style scoped>
.ts {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
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
.muted,
.hint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
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
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
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
/* Form buttons keep their natural width, at the right edge. */
.save .end {
  justify-self: end;
}
/* On its own page the list is the section body, not a card inside it. */
.card.bare {
  padding: 0;
  border: 0;
  background: transparent;
}
.card .dtxt {
  color: var(--danger);
}
.card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.save {
  display: grid;
  gap: var(--space-3);
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
}
.row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 44px;
  padding: var(--space-2) 0;
  border-bottom: 1px solid var(--border);
}
.row:last-child {
  border-bottom: 0;
}
.nm {
  display: grid;
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}
.oid {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.acts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
}
/* Four actions do not fit beside the note at narrow widths: they wrap under it instead. */
.row.tall {
  flex-wrap: wrap;
}
.row.tall .nm {
  flex: 1 1 160px;
}
.busy {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
.picks {
  display: grid;
  gap: var(--space-1);
  max-height: 200px;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
  overflow-wrap: anywhere;
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  margin: 0 0 var(--space-3);
  font-size: var(--fs-sm);
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
  overflow-wrap: anywhere;
}
.verify,
.opts {
  display: grid;
  gap: var(--space-2);
}
.err {
  margin: 0;
  color: var(--danger);
}
.warn {
  margin: 0;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
  font-size: var(--fs-sm);
}
</style>
