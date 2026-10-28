<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Archive, RefreshCw, Tag, Trash2 } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
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

const ts = useTagsStashStore()
const repo = useRepoStore()
onMounted(() => void ts.refresh())

const message = ref('')
const includeUntracked = ref<boolean | 'indeterminate'>(false)

const s = computed(() => repo.status?.summary)
const tracked = computed(() => (s.value ? s.value.staged + s.value.modified + s.value.deleted : 0))
const stashable = computed(() => tracked.value > 0 || (includeUntracked.value === true && (s.value?.untracked ?? 0) > 0))
const bar = computed(() => (ts.running ? primaryBar(ts.running) : undefined))

async function save() {
  if (!stashable.value || ts.busy) return
  if (await ts.save(message.value, includeUntracked.value === true)) message.value = ''
}

// Tags ------------------------------------------------------------------
const tagName = ref('')
const tagNote = ref('')
const taken = computed(() => ts.tags.some((t) => t.name === tagName.value))
const tagError = computed(() => (tagName.value ? (refNameError(tagName.value) ?? (taken.value ? 'A tag with this name exists' : undefined)) : undefined))
const canTag = computed(() => !!tagName.value && !tagError.value && !ts.busy && !!repo.status?.branch.name)
async function createTag() {
  if (!canTag.value) return
  if (await ts.createTag(tagName.value, tagNote.value)) tagName.value = tagNote.value = ''
}
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
    <header class="head">
      <div class="ttl">
        <h1>Tags &amp; Stash</h1>
        <p>name versions, park unfinished work</p>
      </div>
      <IconButton label="Refresh tags and stash" variant="secondary" :disabled="ts.loading" @click="ts.refresh()"><RefreshCw :size="18" /></IconButton>
    </header>

    <div v-if="ts.actionError" class="banner" role="alert">
      <code class="mono">{{ ts.actionError.code }}</code>
      <span>{{ ts.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="ts.actionError = null">Dismiss</Button>
    </div>

    <div class="grid">
      <section class="card" aria-labelledby="tags-h">
        <h2 id="tags-h">Tags</h2>
        <div v-if="!ts.tagsLoaded && !ts.tagsError"><SkeletonRows :count="3" /></div>
        <ErrorState v-else-if="ts.tagsError" :code="ts.tagsError.code" :message="ts.tagsError.message" @retry="ts.refresh()" />
        <EmptyState v-else-if="!ts.tags.length" title="No tags yet" description="A tag names a version, like v1.0 delivery. Tags made elsewhere show up here.">
          <template #icon><Tag :size="24" /></template>
        </EmptyState>
        <ul v-else class="rows" aria-label="Tags">
          <li v-for="t in ts.tags" :key="t.name" class="row" :data-tag="t.name">
            <TagPill :name="t.name" />
            <span class="nm">{{ t.message || (t.annotated ? '' : 'Plain tag') }}</span>
            <span class="oid mono">{{ shortOid(t.oid) }}</span>
            <IconButton :label="`Delete tag ${t.name}`" size="sm" :disabled="ts.busy" @click="askDeleteTag(t.name)"><Trash2 :size="16" /></IconButton>
          </li>
        </ul>
        <form class="save" @submit.prevent="createTag">
          <TextField v-model="tagName" label="Tag name" placeholder="v1.0" :error="tagError ?? undefined" mono />
          <TextField v-model="tagNote" label="Note (optional)" placeholder="Final delivery" />
          <Button type="submit" variant="primary" :disabled="!canTag" :loading="ts.busy && !ts.running">Tag the latest version</Button>
        </form>
      </section>

      <section class="card" aria-labelledby="stash-h">
        <div class="card-head">
          <h2 id="stash-h">Stash</h2>
          <Button v-if="ts.stashes.length" size="sm" variant="secondary" class="dtxt" :disabled="ts.busy" @click="clearOpen = true">Remove all</Button>
        </div>
        <form class="save" @submit.prevent="save">
          <TextField v-model="message" label="Message (optional)" placeholder="WIP lighting" />
          <Checkbox v-model="includeUntracked" label="Include new files" />
          <Button type="submit" variant="primary" :disabled="!stashable || ts.busy" :loading="ts.busy && !ts.running">Stash my changes</Button>
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
              <Button size="sm" variant="secondary" class="dtxt" :disabled="ts.busy" :aria-label="`Drop stash ${e.index}`" @click="askDrop(e.index)">Drop</Button>
            </div>
          </li>
        </ul>
        <p v-if="ts.stashes.length" class="hint">Apply keeps the entry. Pop applies it and removes it.</p>
      </section>
    </div>

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
  gap: var(--space-1);
}
.busy {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
</style>
