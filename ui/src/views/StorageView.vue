<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { HardDrive, RefreshCw } from 'lucide-vue-next'
import { formatBytes } from '@/components/format'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import Button from '@/components/Button.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { formatWhen } from '@/lib/history'
import { useStorageStore } from '@/stores/storage'

const st = useStorageStore()
onMounted(() => void st.refresh())

const s = computed(() => st.stats)
const empty = computed(() => !!s.value && s.value.total_bytes === 0 && s.value.commits === 0)
const ratio = computed(() => (s.value?.compression_ratio ? `${s.value.compression_ratio.toFixed(2)}×` : '–'))
const saved = computed(() => (s.value && s.value.original_bytes > s.value.total_bytes ? formatBytes(s.value.original_bytes - s.value.total_bytes) : '–'))

// How the stored objects divide, as counts.
const kinds = computed(() => {
  const x = s.value
  if (!x) return []
  const rows = [
    { label: 'Chunks', n: x.chunks },
    { label: 'Deltas', n: x.deltas },
    { label: 'Loose objects', n: x.loose_objects },
    { label: 'Pack files', n: x.pack_files },
  ]
  const max = Math.max(1, ...rows.map((r) => r.n))
  return rows.map((r) => ({ ...r, pct: (r.n / max) * 100 }))
})
const span = computed(() => (s.value?.first_commit_time && s.value.last_commit_time ? `${formatWhen(s.value.first_commit_time)} to ${formatWhen(s.value.last_commit_time)}` : '–'))
const n = (x: number) => x.toLocaleString('en-US')
</script>

<template>
  <section class="st" :aria-busy="st.loading || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">Storage</h2>
      </div>
      <Button :disabled="st.loading" @click="st.refresh()"><RefreshCw :size="16" aria-hidden="true" /> Recalculate storage</Button>
    </header>

    <div v-if="!s && !st.error" class="pad"><SkeletonRows :count="6" /></div>
    <div v-else-if="st.error && !s" class="pad">
      <ErrorState :code="st.error.code" :message="st.error.message" @retry="st.refresh()" />
    </div>
    <div v-else-if="empty" class="pad">
      <EmptyState title="Nothing stored yet" description="Save your first version and its size shows up here.">
        <template #icon><HardDrive :size="24" /></template>
      </EmptyState>
    </div>

    <div v-else-if="s" class="body">
      <div v-if="st.error" class="banner" role="alert"><span>{{ st.error.message }}</span></div>
      <ul class="cards" aria-label="Storage summary">
        <li class="card stat"><b>{{ formatBytes(s.total_bytes) }}</b><span>Stored on disk</span></li>
        <li class="card stat"><b>{{ s.original_bytes ? formatBytes(s.original_bytes) : '–' }}</b><span>If every version were a full copy</span></li>
        <li class="card stat"><b>{{ ratio }}</b><span>Saved by sharing and compression</span></li>
        <li class="card stat"><b>{{ saved }}</b><span>Space saved</span></li>
      </ul>

      <div class="grid">
        <section class="card" aria-labelledby="kinds-h">
          <h2 id="kinds-h">What is stored</h2>
          <div v-for="k in kinds" :key="k.label" class="sbar">
            <span>{{ k.label }}</span>
            <ProgressBar :label="k.label" :value="k.pct" size="sm" />
            <span class="num mono">{{ n(k.n) }}</span>
          </div>
        </section>

        <section class="card" aria-labelledby="proj-h">
          <h2 id="proj-h">The project</h2>
          <dl class="kv">
            <dt>Versions</dt><dd class="mono">{{ n(s.commits) }}</dd>
            <dt>Files</dt><dd class="mono">{{ n(s.files) }}</dd>
            <dt>Media files</dt><dd class="mono">{{ n(s.media_files) }}</dd>
            <dt>Branches</dt><dd class="mono">{{ n(s.local_branches) }} here, {{ n(s.remote_branches) }} on remotes</dd>
            <dt>Tags</dt><dd class="mono">{{ n(s.tags) }}</dd>
            <dt>History spans</dt><dd>{{ span }}</dd>
          </dl>
        </section>
      </div>

      <section v-if="s.authors.length" class="card" aria-labelledby="auth-h">
        <h2 id="auth-h">People</h2>
        <table class="t">
          <thead><tr><th scope="col">Name</th><th scope="col">Email</th><th scope="col" class="r">Versions</th></tr></thead>
          <tbody>
            <tr v-for="a in s.authors" :key="a.email + a.name">
              <th scope="row">{{ a.name }}</th>
              <td class="mono">{{ a.email }}</td>
              <td class="mono r">{{ n(a.commits) }}</td>
            </tr>
          </tbody>
        </table>
      </section>
    </div>
  </section>
</template>

<style scoped>
.st {
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
h2 {
  margin: 0;
  font-size: var(--fs-lg);
}
.ttl p {
  margin: 0;
  color: var(--text-muted);
}
.body {
  display: grid;
  gap: var(--space-4);
}
.banner {
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-4);
  margin: 0;
  padding: 0;
  list-style: none;
}
.card {
  display: grid;
  gap: var(--space-3);
  align-content: start;
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.stat {
  gap: var(--space-1);
}
.stat b {
  font-size: var(--fs-2xl);
}
.stat span {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: var(--space-4);
}
.sbar {
  display: grid;
  grid-template-columns: 110px 1fr 80px;
  align-items: center;
  gap: var(--space-3);
}
.num {
  text-align: right;
  color: var(--text-muted);
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-2) var(--space-5);
  margin: 0;
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
}
.t {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-sm);
}
.t tr {
  height: var(--row-h);
}
.t th,
.t td {
  /* 8px at the comfortable 36px row; the 8px shrinks by half of what compact takes off the row. */
  padding: calc(var(--space-2) - (36px - var(--row-h)) / 2) var(--space-3);
  border-bottom: 1px solid var(--border);
  text-align: left;
}
.t thead th {
  color: var(--text-muted);
}
.t tbody th {
  font-weight: 500;
}
.r {
  text-align: right !important;
}
</style>
