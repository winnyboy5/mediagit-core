<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { GitCompare, RefreshCw } from 'lucide-vue-next'
import { api } from '@/api'
import { formatBytes } from '@/components/format'
import Checkbox from '@/components/Checkbox.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import StatusBadge, { type FileStatus } from '@/components/StatusBadge.vue'
import Tabs from '@/components/Tabs.vue'
import { metaRows } from '@/lib/compare'
import { previewKind, type InspectFile } from '@/lib/inspect'
import { baseName } from '@/lib/persist'
import { shortOid } from '@/lib/history'
import { fileKey, useInspectorStore } from '@/stores/inspector'
import { useCompareStore, WORKING } from '@/stores/compare'

const compare = useCompareStore()
const inspector = useInspectorStore()
onMounted(() => {
  if (!compare.commitsLoaded) void compare.loadCommits()
  else void compare.compare()
})

const pickOptions = computed(() => compare.commits.map((c) => ({ value: c.oid, label: `${shortOid(c.oid)} · ${c.summary || '(no message)'}` })))
const toOptions = computed(() => [{ value: WORKING, label: 'Working folder' }, ...pickOptions.value])
// Against the working folder the left side is always the latest version, whatever `from` holds.
const LATEST = 'latest'
const fromOptions = computed(() => (compare.working ? [{ value: LATEST, label: 'Latest saved version' }] : pickOptions.value))
const fromValue = computed({ get: () => (compare.working ? LATEST : compare.from), set: (v: string | undefined) => v && (compare.from = v) })

const firstLoad = computed(() => (!compare.commitsLoaded && !compare.commitsError) || (compare.loading && !compare.report))
const noVersions = computed(() => compare.commitsLoaded && !compare.commits.length)
const sameVersion = computed(() => !compare.working && !!compare.from && compare.from === compare.to)

// The selected file's two sides ---------------------------------------------
const extraction = (f: InspectFile | null) => (f ? inspector.extractions.get(fileKey(f)) : undefined)
const info = (f: InspectFile | null) => (f ? inspector.infos.get(fileKey(f)) : undefined)
const before = computed(() => compare.sides.before)
const after = computed(() => compare.sides.after)
const eb = computed(() => extraction(before.value))
const ea = computed(() => extraction(after.value))
const isImage = computed(() => !!compare.file && previewKind(compare.file.path) === 'image')
const extracting = computed(() => eb.value?.status === 'loading' || ea.value?.status === 'loading')
const extractError = computed(() => eb.value?.error ?? ea.value?.error)
const ready = computed(() => isImage.value && [eb, ea].every((e, i) => !([before, after][i]!.value) || e.value?.status === 'ready'))
const src = (e: { path?: string } | undefined) => (e?.path ? api.fileSrc(e.path) : '')

const beforeLabel = computed(() => `Before · ${shortOid(compare.report?.from ?? '') || 'earlier'}`)
const afterLabel = computed(() => (compare.working ? 'After · Working folder' : `After · ${shortOid(compare.report?.to ?? '')}`))

const onlyChanges = ref<boolean | 'indeterminate'>(true)
const rows = computed(() => {
  const f = compare.file
  if (!f) return []
  const all = metaRows(info(before.value)?.value, info(after.value)?.value, { before: f.old_size, after: f.new_size })
  return onlyChanges.value === true ? all.filter((r) => r.changed) : all
})
const infoLoading = computed(() => [before.value, after.value].some((s) => s && info(s)?.status === 'loading'))

const MODES = [
  { value: 'side', label: 'Side by side' },
  { value: 'onion', label: 'Onion skin' },
  { value: 'swipe', label: 'Swipe' },
]
const mode = ref('side')
const blend = ref(50)
const divider = ref(50)

const dir = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '')
const status = (s: string) => s as FileStatus
const size = (f: { old_size: number | null; new_size: number | null }) => formatBytes(f.new_size ?? f.old_size ?? 0)
</script>

<template>
  <section class="cmp" :aria-busy="compare.loading || extracting || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Compare</h1>
        <p>what changed between two versions</p>
      </div>
      <div class="tools">
        <Select v-model="fromValue" label="From" :options="fromOptions" :disabled="compare.working || !compare.commits.length" />
        <span class="arrow" aria-hidden="true">→</span>
        <Select v-model="compare.to" label="To" :options="toOptions" :disabled="!compare.commits.length" />
        <IconButton label="Refresh comparison" variant="secondary" :disabled="compare.loading || !compare.commits.length" @click="compare.compare()"><RefreshCw :size="18" /></IconButton>
      </div>
    </header>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="6" /></div>
    <div v-else-if="compare.commitsError" class="pad">
      <ErrorState :code="compare.commitsError.code" :message="compare.commitsError.message" @retry="compare.loadCommits()" />
    </div>
    <div v-else-if="noVersions" class="pad">
      <EmptyState title="Nothing to compare" description="Save a version first, then compare it with your working folder or another version.">
        <template #icon><GitCompare :size="24" /></template>
      </EmptyState>
    </div>
    <div v-else-if="compare.error" class="pad">
      <ErrorState :code="compare.error.code" :message="compare.error.message" @retry="compare.compare()" />
    </div>
    <div v-else-if="sameVersion" class="pad">
      <EmptyState title="Pick two different versions" description="Both sides are the same version, so nothing differs." />
    </div>
    <div v-else-if="!compare.files.length" class="pad">
      <EmptyState title="No differences" :description="compare.working ? 'Your working folder matches the latest saved version.' : 'These two versions hold the same files.'">
        <template #icon><GitCompare :size="24" /></template>
      </EmptyState>
    </div>

    <div v-else class="split">
      <ul class="files" aria-label="Changed files">
        <li v-for="f in compare.files" :key="f.path">
          <button type="button" class="file" :class="{ sel: f.path === compare.selectedPath }" :aria-pressed="f.path === compare.selectedPath" :title="f.path" @click="compare.select(f.path)">
            <StatusBadge :status="status(f.status)" />
            <span class="fn"><span class="base">{{ baseName(f.path) }}</span><span v-if="dir(f.path)" class="dir mono">{{ dir(f.path) }}</span></span>
            <span class="sz mono">{{ size(f) }}</span>
          </button>
        </li>
      </ul>

      <div class="pane" data-testid="compare-pane">
        <EmptyState v-if="!compare.file" title="Pick a file" description="Choose a changed file to see both sides." />
        <template v-else>
          <header class="fh">
            <h2>{{ compare.file.path }}</h2>
            <span class="chip" :class="compare.file.binary ? 'bin' : 'txt'">{{ compare.file.binary ? 'Binary file' : 'Text file' }}</span>
          </header>
          <p v-if="!compare.file.binary" class="muted">Line-by-line changes are not shown here yet; sizes and properties are.</p>

          <div v-if="extracting" class="bars" role="status" data-testid="compare-extract">
            <div v-for="(e, i) in [eb, ea]" :key="i">
              <template v-if="[before, after][i] && e?.status === 'loading'">
                <span class="muted">Extracting {{ i === 0 ? 'before' : 'after' }}<template v-if="e.len"> · {{ formatBytes(e.pos) }} of {{ formatBytes(e.len) }}</template></span>
                <ProgressBar :label="`Extracting ${i === 0 ? 'before' : 'after'}`" :value="e.len ? e.pos : null" :max="e.len ?? 100" />
              </template>
            </div>
          </div>
          <p v-else-if="extractError" class="err" role="alert">Could not extract a side of this file: {{ extractError.message }}</p>

          <template v-if="ready">
            <figure v-if="!before || !after" class="one">
              <img :src="src(before ? eb : ea)" :alt="`${before ? 'Before' : 'After'}: ${compare.file.path}`" />
              <figcaption>{{ before ? beforeLabel : afterLabel }} · {{ before ? 'deleted in the later side' : 'new file' }}</figcaption>
            </figure>
            <Tabs v-else v-model="mode" label="Comparison style" :items="MODES">
              <template #default="{ value }">
                <div v-if="value === 'side'" class="two">
                  <figure><img :src="src(eb)" :alt="`Before: ${compare.file.path}`" /><figcaption>{{ beforeLabel }}</figcaption></figure>
                  <figure><img :src="src(ea)" :alt="`After: ${compare.file.path}`" /><figcaption>{{ afterLabel }}</figcaption></figure>
                </div>
                <div v-else-if="value === 'onion'" class="stack-wrap">
                  <div class="stack" data-testid="onion">
                    <img :src="src(eb)" :alt="`Before: ${compare.file.path}`" />
                    <img class="top" :style="{ opacity: blend / 100 }" :src="src(ea)" :alt="`After: ${compare.file.path}`" />
                  </div>
                  <label class="range">Blend before and after<input v-model.number="blend" type="range" min="0" max="100" aria-label="Blend before and after" /></label>
                </div>
                <div v-else class="stack-wrap">
                  <div class="stack swipe" data-testid="swipe">
                    <img :src="src(eb)" :alt="`Before: ${compare.file.path}`" />
                    <img class="top" :style="{ clipPath: `inset(0 0 0 ${divider}%)` }" :src="src(ea)" :alt="`After: ${compare.file.path}`" />
                    <span class="corner l" aria-hidden="true">{{ beforeLabel }}</span>
                    <span class="corner r" aria-hidden="true">After</span>
                    <span class="divider" :style="{ left: `${divider}%` }" aria-hidden="true"><span class="grab" /></span>
                  </div>
                  <label class="range">Move divider<input v-model.number="divider" type="range" min="0" max="100" aria-label="Move divider" /></label>
                </div>
              </template>
            </Tabs>
          </template>
          <p v-else-if="!isImage" class="muted">No picture preview for this kind of file. Compare its size and properties below.</p>

          <section class="meta" aria-labelledby="meta-h">
            <div class="meta-head">
              <h3 id="meta-h">Properties</h3>
              <Checkbox v-model="onlyChanges" label="Only what changed" />
            </div>
            <p v-if="infoLoading" class="muted" role="status">Reading properties…</p>
            <p v-else-if="!rows.length" class="muted">No properties differ.</p>
            <table v-else class="t">
              <thead><tr><th scope="col">Property</th><th scope="col">Before</th><th scope="col">After</th></tr></thead>
              <tbody>
                <tr v-for="r in rows" :key="r.label" :class="{ changed: r.changed }">
                  <th scope="row">{{ r.label }}</th>
                  <td class="mono">{{ r.before }}</td>
                  <td class="mono">{{ r.after }}<span v-if="r.changed" class="sr-only"> (changed)</span></td>
                </tr>
              </tbody>
            </table>
          </section>
        </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
.cmp {
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
h2,
h3 {
  margin: 0;
  font-size: var(--fs-lg);
}
h3 {
  font-size: var(--fs-md);
}
.ttl p,
.muted {
  margin: 0;
  color: var(--text-muted);
}
.tools {
  display: flex;
  align-items: flex-end;
  gap: var(--space-2);
}
.tools :deep(.field) {
  min-width: 220px;
}
.arrow {
  padding-bottom: var(--space-2);
  color: var(--text-muted);
}
.pad {
  overflow-y: auto;
}
.split {
  display: grid;
  flex: 1;
  grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
  gap: var(--space-4);
  min-height: 0;
}
.files {
  margin: 0;
  padding: var(--space-1);
  overflow-y: auto;
  list-style: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.file {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-2);
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
.file:hover {
  background: var(--surface-2);
}
.file.sel {
  background: var(--accent-soft);
}
.fn {
  display: grid;
  min-width: 0;
}
.base {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.dir {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sz {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.pane {
  display: grid;
  gap: var(--space-4);
  align-content: start;
  min-width: 0;
  padding: var(--space-4) var(--space-5);
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.fh {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.fh h2 {
  overflow-wrap: anywhere;
}
.chip {
  flex: none;
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.bars {
  display: grid;
  gap: var(--space-3);
  font-size: var(--fs-sm);
}
.err {
  margin: 0;
  color: var(--danger);
}
.two {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-4);
}
figure {
  display: grid;
  gap: var(--space-2);
  margin: 0;
}
figcaption {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
img {
  display: block;
  width: 100%;
  max-height: 420px;
  object-fit: contain;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--surface-2);
}
.stack-wrap {
  display: grid;
  gap: var(--space-3);
}
.stack {
  display: grid;
}
.stack img {
  grid-area: 1 / 1;
}
.stack .top {
  background: transparent;
}
.range {
  display: grid;
  gap: var(--space-1);
  max-width: 360px;
  font-size: var(--fs-sm);
  color: var(--text-muted);
  /* Neutral thumb: a blue or orange control would tint a colour-critical comparison. */
  accent-color: var(--text-muted);
}
.swipe {
  position: relative;
}
.divider {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--text-muted);
  pointer-events: none;
}
.grab {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 20px;
  height: 36px;
  border: 1px solid var(--text-muted);
  border-radius: var(--radius-pill);
  background: var(--surface);
  box-shadow: var(--shadow-1);
  transform: translate(-50%, -50%);
}
.corner {
  position: absolute;
  top: var(--space-2);
  padding: 2px var(--space-2);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font-size: var(--fs-xs);
  font-weight: 600;
  pointer-events: none;
}
.corner.l {
  left: var(--space-2);
}
.corner.r {
  right: var(--space-2);
}
.meta {
  display: grid;
  gap: var(--space-2);
}
.meta-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.t {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-sm);
}
.t th,
.t td {
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--border);
  text-align: left;
}
.t thead th {
  color: var(--text-muted);
  font-weight: 600;
}
.t tbody th {
  font-weight: 500;
}
.t tr.changed td:last-child {
  background: var(--modified-soft);
}
</style>
