<script setup lang="ts">
import { computed, nextTick, onMounted, ref, useId, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { Filter, History, PanelRight, Search } from 'lucide-vue-next'
import { useFocusRefresh } from '@/lib/focusRefresh'
import BranchPill from '@/components/BranchPill.vue'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import Select from '@/components/Select.vue'
import TextField from '@/components/TextField.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import TagPill from '@/components/TagPill.vue'
import { laneIndex } from '@/lib/graphLayout'
import { formatWhen, graphWidth, shortOid } from '@/lib/history'
import { timeAgo } from '@/lib/persist'
import { useHistoryStore } from '@/stores/history'
import { useRepoStore } from '@/stores/repo'
import { useShellStore } from '@/stores/shell'
import CommitDetail from './history/CommitDetail.vue'
import GraphCell from './history/GraphCell.vue'

const history = useHistoryStore()
const repo = useRepoStore()
const shell = useShellStore()

const SCOPES = [
  { value: 'all', label: 'All branches' },
  { value: 'branch', label: 'This branch only' },
]
const scope = computed({
  get: () => (history.branchOnly ? 'branch' : 'all'),
  set: (v: string) => (history.branchOnly = v === 'branch'),
})

// Filters the CLI applies before paging (`log --author` and friends).
const filtersOpen = ref(false)
// The select cannot hold an empty value, so "every branch" is a named choice.
const EVERY = '*every'
const startValue = computed({ get: () => history.filter.revision || EVERY, set: (v?: string) => (history.filter.revision = !v || v === EVERY ? '' : v) })
const startOptions = computed(() => [
  { value: EVERY, label: 'Every branch' },
  ...history.refs.filter((r) => r.kind === 'branch' || r.kind === 'tag').map((r) => ({ value: r.name, label: `${r.kind === 'tag' ? 'Tag' : 'Branch'} ${r.name}` })),
])

const MAX_LANES = 8
const laneOf = (n: number) => laneIndex(n) as 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7
const searchId = useId()
const uid = useId()
const el = ref<HTMLElement | null>(null)
const rowH = ref(36)
const current = computed(() => repo.status?.branch.name ?? null)

onMounted(() => {
  const h = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--row-h'))
  if (h > 0) rowH.value = h
  void history.reload()
})

useFocusRefresh(() => history.reload())

const virtual = useVirtualizer(
  computed(() => ({
    count: history.visible.length,
    getScrollElement: () => el.value,
    estimateSize: () => rowH.value,
    overscan: 10,
    initialRect: { width: 800, height: 600 },
  })),
)
const items = computed(() => virtual.value.getVirtualItems())
const total = computed(() => virtual.value.getTotalSize())

// Near the end of what is loaded: fetch the next page.
watch(items, (list) => {
  const last = list[list.length - 1]
  if (last && last.index >= history.visible.length - 25) void history.loadMore()
})

const graphCols = computed(() => Math.min(MAX_LANES, history.rows.reduce((n, r) => Math.max(n, r.width), 1)))
const gw = computed(() => graphWidth(graphCols.value))

const selIndex = computed(() => history.visible.findIndex((c) => c.oid === history.selectedOid))
const activeId = computed(() => (selIndex.value >= 0 ? `${uid}-${selIndex.value}` : undefined))

async function key(e: KeyboardEvent) {
  const last = history.visible.length - 1
  const delta = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : e.key === 'PageDown' ? 10 : e.key === 'PageUp' ? -10 : null
  if (delta !== null) history.step(delta)
  else if (e.key === 'Home') void history.select(history.visible[0]?.oid ?? null)
  else if (e.key === 'End') void history.select(history.visible[last]?.oid ?? null)
  else return
  e.preventDefault()
  await nextTick()
  if (selIndex.value >= 0) virtual.value.scrollToIndex(selIndex.value, { align: 'auto' })
}

function click(oid: string) {
  el.value?.focus({ preventScroll: true })
  void history.select(oid)
}

const firstLoad = computed(() => !history.loaded && !history.error)
const empty = computed(() => history.loaded && history.commits.length === 0)
const noMatch = computed(() => history.loaded && history.commits.length > 0 && history.visible.length === 0 && history.done)
const busy = computed(() => history.loading || history.loadingMore)
</script>

<template>
  <section class="history" :aria-busy="busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">History</h2>
      </div>
      <div class="tools">
        <div class="search">
          <label :for="searchId" class="sr-only">Search versions</label>
          <Search :size="16" aria-hidden="true" />
          <input :id="searchId" v-model="history.query" type="search" placeholder="Search versions" autocomplete="off" />
        </div>
        <SegmentedControl v-model="scope" label="Which versions" :options="SCOPES" />
        <Button :aria-pressed="filtersOpen" aria-controls="history-filters" @click="filtersOpen = !filtersOpen">
          <Filter :size="16" aria-hidden="true" /> {{ history.filtering ? 'Filters (on)' : 'Filters' }}
        </Button>
        <IconButton
          v-if="shell.narrow"
          :label="shell.inspectorVisible ? 'Hide details panel' : 'Show details panel'"
          variant="secondary"
          :pressed="shell.inspectorVisible"
          @click="shell.toggleInspector()"
        >
          <PanelRight :size="18" />
        </IconButton>
      </div>
    </header>

    <form v-show="filtersOpen" id="history-filters" class="filters" aria-label="Filter versions" @submit.prevent="history.applyFilter()">
      <Select v-model="startValue" label="Start from" :options="startOptions" />
      <TextField v-model="history.filter.author" label="Saved by" placeholder="e.g. Ana or ana@studio.example" hint="Only versions saved by this person." flag="--author" />
      <TextField v-model="history.filter.grep" label="Message contains" placeholder="e.g. grade pass" hint="Only versions whose message has these words." flag="--grep" />
      <TextField v-model="history.filter.since" label="From date" type="date" hint="Leave empty to go back to the start." flag="--since" />
      <TextField v-model="history.filter.until" label="Until date" type="date" hint="Leave empty to include the newest." flag="--until" />
      <TextField v-model="history.filter.paths" label="Only these files or folders" placeholder="e.g. art/hero.psd, video/shots" hint="Separate with commas" mono />
      <div class="factions">
        <Button type="submit" variant="primary" :disabled="history.loading">Apply filters</Button>
        <Button type="button" :disabled="!history.filtering" @click="history.clearFilter()">Clear</Button>
      </div>
    </form>

    <div v-if="firstLoad" class="pad"><SkeletonRows :count="10" /></div>
    <div v-else-if="history.error && !history.loaded" class="pad">
      <ErrorState :code="history.error.code" :message="history.error.message" @retry="history.reload()" />
    </div>
    <div v-else-if="empty" class="pad">
      <EmptyState title="No saved versions yet" description="Save your first version from Changes and it appears here.">
        <template #icon><History :size="24" /></template>
      </EmptyState>
    </div>

    <div v-else class="split">
      <div class="panel">
        <div v-if="history.error" class="banner" role="alert">
          <code class="mono">{{ history.error.code }}</code>
          <span>{{ history.error.message }}</span>
          <Button size="sm" variant="ghost" @click="history.reload()">Retry</Button>
        </div>
        <p v-if="history.searching && history.visible.length" class="hint">The graph is hidden while you search or filter.</p>
        <div
          ref="el"
          class="list"
          role="listbox"
          tabindex="0"
          aria-label="Versions, newest first"
          :aria-activedescendant="activeId"
          @keydown="key"
        >
          <div class="sizer" :style="{ height: total + 'px' }">
            <div
              v-for="v in items"
              :id="`${uid}-${v.index}`"
              :key="history.visible[v.index]!.oid"
              class="item"
              role="option"
              :aria-selected="history.visible[v.index]!.oid === history.selectedOid"
              :aria-posinset="v.index + 1"
              :aria-setsize="history.visible.length"
              :class="{ sel: history.visible[v.index]!.oid === history.selectedOid }"
              :style="{ height: v.size + 'px', transform: `translateY(${v.start}px)` }"
              @click="click(history.visible[v.index]!.oid)"
            >
              <span v-if="!history.searching" class="gcell" :style="{ width: gw + 'px' }">
                <GraphCell
                  v-if="history.rows[v.index]"
                  :row="history.rows[v.index]!"
                  :height="v.size"
                  :head="history.visible[v.index]!.oid === history.head"
                />
              </span>
              <span class="sum">{{ history.visible[v.index]!.summary || '(no message)' }}</span>
              <span v-if="history.decorations.get(history.visible[v.index]!.oid)" class="pills">
                <template v-for="(p, pi) in history.decorations.get(history.visible[v.index]!.oid)!.slice(0, 2)" :key="p.kind + p.name">
                  <TagPill v-if="p.kind === 'tag'" :name="p.name" />
                  <BranchPill v-else :name="p.name" :current="p.name === current" :lane="laneOf(history.rows[v.index]?.col ?? pi)" />
                </template>
                <span v-if="history.decorations.get(history.visible[v.index]!.oid)!.length > 2" class="more">
                  +{{ history.decorations.get(history.visible[v.index]!.oid)!.length - 2 }}
                </span>
              </span>
              <span v-if="history.visible[v.index]!.oid === history.head" class="sr-only">(current version)</span>
              <span class="by">{{ history.visible[v.index]!.author.name }}</span>
              <time class="when" :datetime="new Date(history.visible[v.index]!.time * 1000).toISOString()" :title="formatWhen(history.visible[v.index]!.time)">
                {{ timeAgo(history.visible[v.index]!.time * 1000) }}
              </time>
              <span class="oid mono">{{ shortOid(history.visible[v.index]!.oid) }}</span>
            </div>
          </div>
          <p v-if="noMatch" class="none">No versions match{{ history.searching ? ` "${history.query}"` : '' }}.</p>
          <p v-else-if="history.loadingMore" class="foot" role="status">Loading older versions…</p>
          <p v-else-if="history.moreError" class="foot err" role="alert">
            Could not load older versions: {{ history.moreError.message }}
            <Button size="sm" variant="ghost" @click="history.loadMore()">Retry</Button>
          </p>
          <p v-else-if="history.done && history.visible.length" class="foot">That is the whole history.</p>
        </div>
      </div>

      <CommitDetail />
    </div>
  </section>
</template>

<style scoped>
.history {
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
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3) var(--space-4);
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
.filters {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-3) var(--space-4);
  align-items: start;
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.factions {
  display: flex;
  align-self: end;
  gap: var(--space-2);
}
.search {
  position: relative;
  display: flex;
  align-items: center;
  color: var(--text-muted);
}
.search svg {
  position: absolute;
  left: var(--space-3);
  pointer-events: none;
}
.search input {
  width: 220px;
  height: var(--control-h);
  padding: 0 var(--space-3) 0 36px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
  font: inherit;
}
.search input::placeholder {
  color: var(--text-muted);
}
.search input:focus-visible {
  outline: var(--focus-ring);
  outline-offset: var(--focus-offset);
}
.pad {
  overflow-y: auto;
}
/* List on the left, the selected version in a column on the right. */
.split {
  display: flex;
  flex: 1;
  gap: var(--space-4);
  min-height: 0;
}
.split > .panel {
  flex: 1 1 0;
  min-width: 0;
}
.split > :deep(.detail) {
  flex: none;
  width: clamp(320px, 38%, 460px);
}
@media (max-width: 900px) {
  .split {
    flex-direction: column;
  }
  .split > :deep(.detail) {
    width: auto;
    flex: 1 1 0;
    min-height: 240px;
  }
}
.panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.list {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: auto;
}
.sizer {
  position: relative;
  width: 100%;
}
.item {
  position: absolute;
  top: 0;
  left: 0;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  padding-right: var(--space-4);
  cursor: default;
}
.item:hover {
  background: var(--surface-2);
}
.item.sel {
  background: var(--accent-soft);
}
.list:focus-visible .item.sel {
  outline: var(--focus-ring);
  outline-offset: -2px;
}
.gcell {
  display: flex;
  flex: none;
  align-items: center;
  height: 100%;
  overflow: hidden;
}
.sum {
  font-weight: 600;
  flex: 1 1 0;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.item:not(:has(.gcell)) .sum {
  margin-left: var(--space-4);
}
.pills {
  display: flex;
  flex: none;
  align-items: center;
  gap: var(--space-1);
  max-width: 45%;
  overflow: hidden;
}
.more {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.by,
.when,
.oid {
  flex: none;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.by {
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.when {
  min-width: 64px;
  text-align: right;
}
.oid {
  min-width: 56px;
}
.hint,
.none,
.foot {
  margin: 0;
  padding: var(--space-3) var(--space-4);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.none {
  text-align: center;
}
.foot {
  text-align: center;
}
.foot.err {
  color: var(--danger);
}
.banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border-bottom: 1px solid var(--danger);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
@media (max-width: 1180px) {
  .oid,
  .by {
    display: none;
  }
}
</style>
