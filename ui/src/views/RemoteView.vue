<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { RouterLink } from 'vue-router'
import { ChevronRight, Cloud, File, Folder, RefreshCw } from 'lucide-vue-next'
import { api, toApiError } from '@/api'
import { formatBytes } from '@/components/format'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { previewKind } from '@/lib/inspect'
import { breadcrumbs } from '@/lib/remoteBrowse'
import { pickFolder } from '@/shell/useOpenRepo'
import { useBranchesStore } from '@/stores/branches'
import { primaryBar, useOpsStore } from '@/stores/ops'
import { DEFAULT_REF, useRemoteBrowserStore } from '@/stores/remoteBrowser'
import { useSyncStore } from '@/stores/sync'
import { useTagsStashStore } from '@/stores/tagsStash'

const rb = useRemoteBrowserStore()
const sync = useSyncStore()
const branches = useBranchesStore()
const tags = useTagsStashStore()
const ops = useOpsStore()

onMounted(async () => {
  void branches.refresh()
  void tags.refresh()
  if (!sync.loaded) await sync.refresh()
  if (rb.remote && !rb.listing && !rb.loading) void rb.load()
})

const remoteOptions = computed(() => sync.remotes.map((r) => ({ value: r.name, label: r.name })))
const remoteValue = computed({ get: () => rb.remote?.name, set: (v: string | undefined) => v && (rb.remoteName = v) })
const refOptions = computed(() => [
  { value: DEFAULT_REF, label: 'Default branch' },
  ...branches.list.map((b) => ({ value: b.name, label: b.name })),
  ...tags.tags.filter((t) => !branches.list.some((b) => b.name === t.name)).map((t) => ({ value: t.name, label: `${t.name} (tag)` })),
])
// A change of remote or version starts again from the root.
watch([remoteValue, () => rb.ref], () => void rb.load())

const noRemote = computed(() => sync.loaded && !sync.remotes.length)
const firstLoad = computed(() => (!sync.loaded && !sync.error) || (rb.loading && !rb.listing))
const crumbs = computed(() => breadcrumbs(rb.remote?.name ?? 'remote', rb.path))
const sel = computed(() => rb.selected)
const kind = computed(() => (sel.value ? previewKind(sel.value.path) : 'none'))
const bar = computed(() => (rb.active ? primaryBar(rb.active) : undefined))
const prev = computed(() => rb.preview)
const prevBar = computed(() => (prev.value?.status === 'loading' ? prev.value : null))
const src = computed(() => (prev.value?.path ? api.fileSrc(prev.value.path) : ''))

async function save() {
  try {
    const dir = await pickFolder('Save file to')
    if (dir) await rb.download(dir)
  } catch (e) {
    rb.downloadError = toApiError(e)
  }
}
const open = (e: { kind: string; path: string }) => void (e.kind === 'dir' ? rb.open(e.path) : undefined)
</script>

<template>
  <section class="rv" :aria-busy="rb.loading || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Remote Browser</h1>
        <p>look inside a server repository without cloning</p>
      </div>
      <div class="tools">
        <Select v-model="remoteValue" label="Remote" :options="remoteOptions" :disabled="!sync.remotes.length" />
        <Select v-model="rb.ref" label="Version" :options="refOptions" :disabled="!sync.remotes.length" />
        <IconButton label="Reload listing" variant="secondary" :disabled="rb.loading || !rb.remote" @click="rb.open(rb.path)"><RefreshCw :size="18" /></IconButton>
      </div>
    </header>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="8" /></div>
    <div v-else-if="sync.error && !sync.loaded" class="pad">
      <ErrorState :code="sync.error.code" :message="sync.error.message" @retry="sync.refresh()" />
    </div>
    <div v-else-if="noRemote" class="pad">
      <EmptyState title="No remote configured" description="Add your studio server on the Sync page to browse its folders here.">
        <template #icon><Cloud :size="24" /></template>
        <RouterLink to="/sync" class="link">Go to Sync</RouterLink>
      </EmptyState>
    </div>
    <div v-else-if="rb.error && !rb.listing" class="pad">
      <ErrorState :code="rb.error.code" :message="rb.error.message" @retry="rb.open(rb.path)" />
    </div>

    <div v-else-if="rb.listing" class="split">
      <div class="panel">
        <nav class="crumbs" aria-label="Folder path">
          <template v-for="(c, i) in crumbs" :key="c.path">
            <ChevronRight v-if="i" :size="14" aria-hidden="true" />
            <button type="button" class="crumb mono" :aria-current="i === crumbs.length - 1 ? 'location' : undefined" :disabled="rb.loading" @click="rb.open(c.path)">{{ c.label }}</button>
          </template>
          <span class="muted ref mono">{{ rb.listing.ref }}</span>
        </nav>
        <div v-if="rb.error" class="banner" role="alert">
          <span>{{ rb.error.message }}</span>
          <Button size="sm" variant="ghost" @click="rb.open(rb.path)">Retry</Button>
        </div>
        <p v-if="!rb.entries.length" class="none">This folder is empty.</p>
        <ul v-else class="rows" aria-label="Remote folders and files">
          <li v-for="e in rb.entries" :key="e.path">
            <button
              type="button"
              class="row"
              :class="{ sel: sel?.path === e.path }"
              :data-entry="e.path"
              :aria-pressed="e.kind === 'file' ? sel?.path === e.path : undefined"
              :aria-label="e.kind === 'dir' ? `Open folder ${e.name}` : e.name"
              :disabled="rb.loading"
              @click="e.kind === 'dir' ? open(e) : rb.select(e)"
            >
              <Folder v-if="e.kind === 'dir'" :size="18" aria-hidden="true" />
              <File v-else :size="18" aria-hidden="true" />
              <span class="nm">{{ e.name }}</span>
              <span class="sz mono">{{ e.size != null ? formatBytes(e.size) : '' }}</span>
            </button>
          </li>
        </ul>
        <p class="note">Download saves one file. To get everything, clone from Welcome.</p>
      </div>

      <aside class="pane" aria-label="Selected file" data-testid="remote-pane">
        <EmptyState v-if="!sel" title="Pick a file" description="Choose a file to preview it or save a copy." />
        <template v-else>
          <h2 class="mono fn">{{ sel.path }}</h2>
          <p class="muted">{{ sel.size != null ? formatBytes(sel.size) : 'Size unknown' }} · {{ rb.listing.ref }}</p>

          <div v-if="prevBar" class="bars" role="status" data-testid="preview-progress">
            <span class="muted">Loading preview<template v-if="prevBar.len"> · {{ formatBytes(prevBar.pos) }} of {{ formatBytes(prevBar.len) }}</template></span>
            <ProgressBar label="Loading preview" :value="prevBar.len ? prevBar.pos : null" :max="prevBar.len ?? 100" />
          </div>
          <p v-else-if="prev?.status === 'error'" class="err" role="alert">Could not load the preview: {{ prev.error?.message }}</p>
          <template v-else-if="prev?.status === 'ready'">
            <img v-if="kind === 'image'" :src="src" :alt="`Preview of ${sel.name}`" class="media" />
            <video v-else-if="kind === 'video'" :src="src" controls preload="metadata" class="media" :aria-label="`Preview of ${sel.name}`" />
            <audio v-else-if="kind === 'audio'" :src="src" controls :aria-label="`Preview of ${sel.name}`" />
          </template>
          <p v-else-if="kind === 'none'" class="muted">No preview for this kind of file. You can still save a copy.</p>

          <div v-if="rb.active" class="bars" role="status" data-testid="download-progress">
            <span>{{ rb.active.label }}<template v-if="bar && bar.len"> · {{ formatBytes(bar.pos) }} of {{ formatBytes(bar.len) }}</template></span>
            <ProgressBar :label="rb.active.label" :value="bar && bar.len ? bar.pos : null" :max="bar?.len ?? 100" />
            <Button size="sm" variant="danger" @click="ops.cancel(rb.active.id)">Cancel</Button>
          </div>
          <Button v-else variant="primary" :loading="rb.downloading" @click="save">Download to…</Button>
          <p v-if="rb.downloadError" class="err" role="alert">{{ rb.downloadError.message }}</p>
        </template>
      </aside>
    </div>
  </section>
</template>

<style scoped>
.rv {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
}
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-3) var(--space-4);
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
  font-size: var(--fs-md);
  overflow-wrap: anywhere;
}
.ttl p,
.muted,
.note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.ttl p {
  font-size: var(--fs-md);
}
.tools {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
}
.tools :deep(.field) {
  min-width: 180px;
}
.pad {
  overflow-y: auto;
}
.link {
  color: var(--accent);
  font-weight: 600;
}
.split {
  display: grid;
  flex: 1;
  grid-template-columns: minmax(0, 1fr) minmax(260px, 380px);
  gap: var(--space-4);
  min-height: 0;
}
.panel,
.pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-height: 0;
  padding: var(--space-4) var(--space-5);
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.crumbs {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1);
}
.crumb {
  padding: 2px var(--space-2);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--accent);
  font-size: var(--fs-sm);
  cursor: pointer;
}
.crumb[aria-current] {
  color: var(--text);
  font-weight: 600;
}
.crumb:hover:not([aria-current]) {
  background: var(--accent-soft);
}
.ref {
  margin-left: auto;
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
}
.row {
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  min-height: var(--row-h);
  padding: var(--space-1) var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.row:hover {
  background: var(--surface-2);
}
.row.sel {
  background: var(--accent-soft);
}
.nm {
  overflow-wrap: anywhere;
}
.sz {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.none {
  margin: 0;
  color: var(--text-muted);
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
.fn {
  font-weight: 600;
}
.media {
  display: block;
  width: 100%;
  max-height: 320px;
  object-fit: contain;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.bars {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
.err {
  margin: 0;
  color: var(--danger);
  font-size: var(--fs-sm);
}
</style>
