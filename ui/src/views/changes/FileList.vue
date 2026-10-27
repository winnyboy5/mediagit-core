<script setup lang="ts">
import { computed, nextTick, onMounted, ref, useId } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { ChevronDown, ChevronRight, Folder } from 'lucide-vue-next'
import FileRow from '@/components/FileRow.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { formatBytes } from '@/components/format'
import type { Row } from '@/lib/fileTree'

export interface Mods {
  ctrl?: boolean
  shift?: boolean
}

const props = defineProps<{
  rows: Row[]
  /** `group:path` keys, same shape as the changes store. */
  selected: ReadonlySet<string>
  group: string
  label: string
  /** File currently being staged: number = determinate %, null = indeterminate. */
  progress?: { path: string; value: number | null }
  /** No status badges (a tree of files at a version has no change status). */
  plain?: boolean
}>()
const emit = defineEmits<{
  select: [path: string, mods: Mods]
  toggle: [path: string]
  selectAll: []
}>()

const el = ref<HTMLElement | null>(null)
const rowH = ref(36)
const uid = useId()
const active = ref<string | null>(null)

onMounted(() => {
  const h = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--row-h'))
  if (h > 0) rowH.value = h
})

const virtual = useVirtualizer(
  computed(() => ({
    count: props.rows.length,
    getScrollElement: () => el.value,
    estimateSize: () => rowH.value,
    overscan: 8,
    // Real layout is unavailable in jsdom-style tests; fall back to a fixed window.
    initialRect: { width: 800, height: 600 },
  })),
)
const items = computed(() => virtual.value.getVirtualItems())
const total = computed(() => virtual.value.getTotalSize())

const key = (r: Row) => `${props.group}:${r.node.path}`
const idx = computed(() => props.rows.findIndex((r) => r.node.path === active.value))
const activeId = computed(() => (idx.value >= 0 ? `${uid}-${idx.value}` : undefined))

function click(e: MouseEvent, r: Row) {
  active.value = r.node.path
  el.value?.focus({ preventScroll: true })
  emit('select', r.node.path, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })
}

async function go(to: number, mods: Mods) {
  const i = Math.max(0, Math.min(props.rows.length - 1, to))
  const r = props.rows[i]
  if (!r) return
  active.value = r.node.path
  virtual.value.scrollToIndex(i)
  emit('select', r.node.path, mods)
  await nextTick()
}

function onKey(e: KeyboardEvent) {
  const r = props.rows[idx.value]
  const mods = { shift: e.shiftKey }
  const cur = idx.value < 0 ? 0 : idx.value
  switch (e.key) {
    case 'ArrowDown':
      void go(idx.value < 0 ? 0 : cur + 1, mods)
      break
    case 'ArrowUp':
      void go(cur - 1, mods)
      break
    case 'Home':
      void go(0, mods)
      break
    case 'End':
      void go(props.rows.length - 1, mods)
      break
    case 'ArrowRight':
      if (r?.node.isDir && !r.expanded) emit('toggle', r.node.path)
      break
    case 'ArrowLeft':
      if (r?.node.isDir && r.expanded) emit('toggle', r.node.path)
      break
    case 'Enter':
      if (r?.node.isDir) emit('toggle', r.node.path)
      break
    case ' ':
      if (r) emit('select', r.node.path, { ctrl: true })
      break
    case 'a':
    case 'A':
      if (!(e.ctrlKey || e.metaKey)) return
      emit('selectAll')
      break
    default:
      return
  }
  e.preventDefault()
}
</script>

<template>
  <div
    ref="el"
    class="list"
    role="tree"
    tabindex="0"
    aria-multiselectable="true"
    :aria-label="label"
    :aria-activedescendant="activeId"
    @keydown="onKey"
  >
    <div class="sizer" :style="{ height: total + 'px' }">
      <div
        v-for="v in items"
        :id="`${uid}-${v.index}`"
        :key="rows[v.index]!.node.path"
        class="item"
        role="treeitem"
        :aria-selected="selected.has(key(rows[v.index]!))"
        :aria-expanded="rows[v.index]!.node.isDir ? rows[v.index]!.expanded : undefined"
        :aria-level="rows[v.index]!.node.depth + 1"
        :aria-posinset="v.index + 1"
        :aria-setsize="rows.length"
        :class="{ cur: active === rows[v.index]!.node.path }"
        :style="{ height: v.size + 'px', transform: `translateY(${v.start}px)` }"
        @click="click($event, rows[v.index]!)"
      >
        <div class="indent" :style="{ paddingLeft: rows[v.index]!.node.depth * 20 + 'px' }">
          <template v-if="rows[v.index]!.node.isDir">
            <div class="dir" :class="{ selected: selected.has(key(rows[v.index]!)) }">
              <span class="chev" aria-hidden="true" @click.stop="emit('toggle', rows[v.index]!.node.path)">
                <ChevronDown v-if="rows[v.index]!.expanded" :size="16" />
                <ChevronRight v-else :size="16" />
              </span>
              <Folder :size="18" class="ficon" aria-hidden="true" />
              <span class="nm">{{ rows[v.index]!.node.name }}/</span>
              <span class="cnt">{{ rows[v.index]!.node.count }} {{ rows[v.index]!.node.count === 1 ? 'file' : 'files' }}</span>
              <span v-if="rows[v.index]!.node.size != null" class="sz mono">{{ formatBytes(rows[v.index]!.node.size!) }}</span>
              <StatusBadge v-if="!plain" :status="rows[v.index]!.node.status" />
            </div>
          </template>
          <FileRow
            v-else
            :path="rows[v.index]!.node.name"
            :size="rows[v.index]!.node.size ?? undefined"
            :status="plain ? undefined : rows[v.index]!.node.status"
            :selected="selected.has(key(rows[v.index]!))"
            :progress="progress && progress.path === rows[v.index]!.node.path ? progress.value : undefined"
            :class="{ nested: rows[v.index]!.node.depth > 0 }"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
  width: 100%;
  cursor: default;
}
.cur .dir,
.cur :deep(.row) {
  outline: var(--focus-ring);
  outline-offset: -2px;
}
.list:not(:focus-visible) .cur .dir,
.list:not(:focus-visible) .cur :deep(.row) {
  outline: none;
}
.indent {
  height: 100%;
}
.indent :deep(.row.nested) {
  margin-left: var(--space-5);
}
.dir {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  height: var(--row-h);
  padding: 0 var(--space-4);
  border-radius: var(--radius-control);
}
.dir:hover {
  background: var(--surface-2);
}
.dir.selected {
  background: var(--accent-soft);
}
.chev {
  display: grid;
  place-items: center;
  width: 20px;
  height: 20px;
  color: var(--text-muted);
}
.ficon {
  color: var(--text-muted);
}
.nm {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cnt,
.sz {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
