<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { PanelRightClose } from 'lucide-vue-next'
import FileIcon from '@/components/FileIcon.vue'
import IconButton from '@/components/IconButton.vue'
import { formatBytes } from '@/components/format'
import { extOf, mediaKind, type MediaKind } from '@/components/media'
import StatusBadge from '@/components/StatusBadge.vue'
import type { InspectFile } from '@/lib/inspect'
import { shortOid } from '@/lib/history'
import { useChangesStore } from '@/stores/changes'
import { useHistoryStore } from '@/stores/history'
import { useShellStore } from '@/stores/shell'
import InspectorHistory from './inspector/InspectorHistory.vue'
import InspectorLock from './inspector/InspectorLock.vue'
import InspectorMeta from './inspector/InspectorMeta.vue'
import InspectorPreview from './inspector/InspectorPreview.vue'

const changes = useChangesStore()
const history = useHistoryStore()
const shell = useShellStore()
const route = useRoute()
const inHistory = computed(() => route.path === '/history')

const KIND_LABEL: Record<MediaKind, string> = {
  image: 'Image',
  video: 'Video',
  audio: 'Audio',
  model3d: '3D model',
  document: 'Document',
  vfx: 'VFX and compositing',
  other: 'File',
}
const STATUS_LABEL = { added: 'Added', modified: 'Modified', deleted: 'Deleted', locked: 'Locked', untracked: 'New, not staged', conflict: 'Conflict' } as const

// History describes one file of the selected version; Changes describes the working copy.
const files = computed<InspectFile[]>(() =>
  inHistory.value
    ? history.inspectFile ? [history.inspectFile] : []
    : changes.selectedFiles.map((f) => ({ ...f, rev: null })),
)
const one = computed(() => (files.value.length === 1 ? files.value[0]! : null))
const name = computed(() => one.value?.path.split('/').pop() ?? '')

const knownSize = computed(() => {
  const sized = files.value.filter((f) => f.size != null)
  return sized.length ? sized.reduce((n, f) => n + (f.size ?? 0), 0) : null
})

const breakdown = computed(() => {
  const by = new Map<MediaKind, { n: number; sample: string }>()
  for (const f of files.value) {
    const k = mediaKind(f.path).kind
    const cur = by.get(k)
    if (cur) cur.n++
    else by.set(k, { n: 1, sample: f.path })
  }
  return [...by.entries()].map(([kind, v]) => ({ kind, ...v })).sort((a, b) => b.n - a.n)
})
</script>

<template>
  <aside class="insp" aria-label="Details" data-testid="inspector">
    <header>
      <h2>Details</h2>
      <IconButton label="Hide details panel" size="sm" @click="shell.toggleInspector()"><PanelRightClose :size="18" /></IconButton>
    </header>

    <div v-if="!files.length" class="none">
      <template v-if="inHistory">
        <p>Choose a file under "Files at this version" to see it here.</p>
        <p class="hint">You see the file as it was saved in that version.</p>
      </template>
      <template v-else>
        <p>Select a file to see its details.</p>
        <p class="hint">Hold Ctrl or Shift to select several.</p>
      </template>
    </div>

    <template v-else-if="one">
      <div class="title">
        <FileIcon :path="one.path" />
        <div class="t">
          <strong class="mono nm" :title="one.path">{{ name }}</strong>
          <span class="pth mono">{{ one.path }}</span>
        </div>
      </div>

      <slot name="preview" :file="one"><InspectorPreview :file="one" /></slot>

      <section aria-labelledby="insp-details">
        <h3 id="insp-details">Details</h3>
        <dl>
          <dt>Type</dt>
          <dd>{{ KIND_LABEL[mediaKind(one.path).kind] }}<template v-if="extOf(one.path)"> · .{{ extOf(one.path) }}</template></dd>
          <dt>Size</dt>
          <dd class="mono">{{ one.size != null ? formatBytes(one.size) : 'Not reported' }}</dd>
          <template v-if="one.status">
            <dt>Status</dt>
            <dd class="st"><StatusBadge :status="one.status" /> {{ STATUS_LABEL[one.status] }}</dd>
          </template>
          <template v-if="one.group">
            <dt>Section</dt>
            <dd>{{ one.group === 'staged' ? 'Staged for the next version' : 'Not staged' }}</dd>
          </template>
          <template v-if="one.rev">
            <dt>Version</dt>
            <dd class="mono">{{ shortOid(one.rev) }}</dd>
          </template>
        </dl>
      </section>

      <slot name="metadata" :file="one"><InspectorMeta :file="one" /></slot>
      <slot name="lock" :file="one"><InspectorLock :file="one" /></slot>
      <slot name="history" :file="one"><InspectorHistory :file="one" /></slot>
    </template>

    <template v-else>
      <section aria-labelledby="insp-sel" data-testid="selection-summary">
        <h3 id="insp-sel">Selection</h3>
        <p class="big">{{ files.length }} files selected</p>
        <dl>
          <dt>Total size</dt>
          <dd class="mono">{{ knownSize != null ? formatBytes(knownSize) : 'Not reported' }}</dd>
        </dl>
      </section>
      <section aria-labelledby="insp-types">
        <h3 id="insp-types">By type</h3>
        <ul class="types">
          <li v-for="b in breakdown" :key="b.kind">
            <FileIcon :path="b.sample" />
            <span class="k">{{ KIND_LABEL[b.kind] }}</span>
            <span class="n mono">{{ b.n }}</span>
          </li>
        </ul>
      </section>
    </template>
  </aside>
</template>

<style scoped>
.insp {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  width: var(--inspector-w);
  flex: none;
  padding: var(--space-4) var(--space-5);
  overflow-y: auto;
  border-left: 1px solid var(--border);
  background: var(--surface);
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
h2 {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 600;
}
h3 {
  margin: 0 0 var(--space-2);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.none {
  display: grid;
  gap: var(--space-1);
  padding: var(--space-6) 0;
  color: var(--text-muted);
}
.none p {
  margin: 0;
}
.hint {
  font-size: var(--fs-sm);
}
.title {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.t {
  display: grid;
  min-width: 0;
}
.nm {
  overflow-wrap: anywhere;
  font-weight: 600;
}
.pth {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
dl {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-2) var(--space-4);
  margin: 0;
}
dt {
  color: var(--text-muted);
}
dd {
  margin: 0;
  text-align: right;
}
.st {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-2);
}
.big {
  margin: 0 0 var(--space-3);
  font-size: var(--fs-xl);
  font-weight: 600;
}
.types {
  display: grid;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}
.types li {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.k {
  flex: 1;
}
.n {
  color: var(--text-muted);
}
</style>
