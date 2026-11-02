<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import ErrorState from '@/components/ErrorState.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import Textarea from '@/components/Textarea.vue'
import TextField from '@/components/TextField.vue'
import { primaryBar } from '@/stores/ops'
import { useLookStore, useRepoSettingsStore, type Density, type Theme } from '@/stores/settings'

const look = useLookStore()
const rs = useRepoSettingsStore()
onMounted(() => void rs.refresh())

const THEMES = [
  { value: 'system', label: 'Match system' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]
const DENSITIES = [
  { value: 'comfortable', label: 'Comfortable' },
  { value: 'compact', label: 'Compact' },
]
const theme = computed({ get: () => look.theme, set: (v: string) => (look.theme = v as Theme) })
const density = computed({ get: () => look.density, set: (v: string) => (look.density = v as Density) })

// Author ----------------------------------------------------------------
const name = ref('')
const email = ref('')
watch(
  () => rs.entries,
  () => {
    name.value = rs.value('author.name') ?? ''
    email.value = rs.value('author.email') ?? ''
  },
  { immediate: true },
)
const emailError = computed(() => (email.value && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.value) ? 'Use an address like you@example.com' : undefined))
const dirty = computed(() => name.value !== (rs.value('author.name') ?? '') || email.value !== (rs.value('author.email') ?? ''))
const canSave = computed(() => !!name.value.trim() && !!email.value && !emailError.value && dirty.value && !rs.busy)
const saveAuthor = () => canSave.value && rs.setAuthor(name.value.trim(), email.value.trim())

// Sparse checkout -----------------------------------------------------------
const patterns = ref('')
const patternMode = ref<boolean | 'indeterminate'>(false)
watch(
  () => rs.sparse,
  (s) => {
    patterns.value = s?.patterns.join('\n') ?? ''
    patternMode.value = s?.mode === 'pattern'
  },
  { immediate: true },
)
const list = computed(() => patterns.value.split('\n').map((p) => p.trim()).filter(Boolean))
const sparseError = computed(() => (list.value.some((p) => p.startsWith('-') || p.includes('..') || p.includes('\\')) ? 'Patterns cannot start with a dash or contain ".." or backslashes' : undefined))
const applyOpen = ref(false)
const disableOpen = ref(false)
const bar = computed(() => (rs.running ? primaryBar(rs.running) : undefined))
const rows = computed(() => rs.entries.filter((e) => e.value !== null))
</script>

<template>
  <section class="set" :aria-busy="rs.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h1>Settings</h1>
        <p>how MediaGit looks and who you are</p>
      </div>
    </header>

    <div v-if="rs.actionError" class="banner" role="alert">
      <code class="mono">{{ rs.actionError.code }}</code>
      <span>{{ rs.actionError.message }}</span>
      <Button size="sm" variant="ghost" @click="rs.actionError = null">Dismiss</Button>
    </div>

    <div class="grid">
      <section class="card" aria-labelledby="look-h" data-testid="appearance">
        <h2 id="look-h">Appearance</h2>
        <div class="field"><span class="lbl" id="theme-l">Theme</span><SegmentedControl v-model="theme" label="Theme" :options="THEMES" /></div>
        <div class="field"><span class="lbl" id="dens-l">Row density</span><SegmentedControl v-model="density" label="Row density" :options="DENSITIES" /></div>
        <p class="muted">Kept on this computer only.</p>
      </section>

      <section class="card" aria-labelledby="id-h" data-testid="identity">
        <h2 id="id-h">Your name on saved versions</h2>
        <div v-if="!rs.loaded && !rs.error"><SkeletonRows :count="2" /></div>
        <ErrorState v-else-if="rs.error && !rs.loaded" :code="rs.error.code" :message="rs.error.message" @retry="rs.refresh()" />
        <form v-else class="form" @submit.prevent="saveAuthor">
          <TextField v-model="name" label="Name" placeholder="Mira Okafor" />
          <TextField v-model="email" label="Email" placeholder="mira@studio.example" :error="emailError" />
          <p class="muted">Also used to recognise your file locks.</p>
          <Button type="submit" variant="primary" :disabled="!canSave" :loading="rs.busy && !rs.running">Save</Button>
        </form>
      </section>

      <section class="card wide" aria-labelledby="cfg-h" data-testid="config">
        <h2 id="cfg-h">All settings</h2>
        <ErrorState v-if="rs.error && !rs.loaded" :code="rs.error.code" :message="rs.error.message" @retry="rs.refresh()" />
        <p v-else-if="!rows.length && rs.loaded" class="muted">Nothing has been changed from the defaults.</p>
        <table v-else-if="rows.length" class="t">
          <thead><tr><th scope="col">Setting</th><th scope="col">Value</th><th scope="col"><span class="sr-only">Actions</span></th></tr></thead>
          <tbody>
            <tr v-for="e in rows" :key="e.key">
              <th scope="row" class="mono">{{ e.key }}</th>
              <td class="mono">{{ e.value }}</td>
              <td class="r"><Button size="sm" variant="ghost" :disabled="rs.busy" :aria-label="`Reset ${e.key}`" @click="rs.unsetKey(e.key)">Reset</Button></td>
            </tr>
          </tbody>
        </table>
      </section>

      <section class="card wide" aria-labelledby="sp-h" data-testid="sparse">
        <h2 id="sp-h">Keep only some files on disk</h2>
        <p class="muted">For a big project, choose folders (one per line) to keep here. Everything else stays in the project and comes back when you add it.</p>
        <p v-if="rs.sparse" class="state">{{ rs.sparse.enabled ? `On, ${rs.sparse.mode === 'pattern' ? 'by pattern' : 'by folder'}` : 'Off: every file is on disk' }}</p>
        <Textarea v-model="patterns" label="Folders or patterns" :rows="5" :error="sparseError" hint="One per line, like art/ or video/shots/" />
        <Checkbox v-model="patternMode" label="Treat lines as patterns, not folders" />
        <div v-if="rs.running" class="bar" role="status" data-testid="sparse-progress">
          <span>{{ rs.running.label }}<template v-if="bar && bar.len != null"> · {{ bar.pos }} / {{ bar.len }} files</template></span>
          <ProgressBar :label="rs.running.label" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
        </div>
        <div class="acts">
          <Button variant="primary" :disabled="!list.length || !!sparseError || rs.busy" @click="applyOpen = true">Apply…</Button>
          <Button :disabled="!rs.sparse?.enabled || rs.busy" @click="disableOpen = true">Keep every file…</Button>
        </div>
      </section>
    </div>

    <AlertDialog
      v-model:open="applyOpen"
      title="Change which files are on disk?"
      description="Files outside your choice are removed from this computer's folder, and files inside it are brought back. They stay safe in the project. Unsaved changes to a file that is removed are not kept."
      confirm-label="Apply"
      @confirm="rs.applySparse(list, patternMode === true)"
    />
    <AlertDialog
      v-model:open="disableOpen"
      title="Keep every file on disk?"
      description="Every file in the project is brought back to this computer's folder. For a big project that can take a while and a lot of space."
      confirm-label="Bring everything back"
      @confirm="rs.disableSparse()"
    />
  </section>
</template>

<style scoped>
.set {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
  overflow-y: auto;
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
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.ttl p {
  font-size: var(--fs-md);
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
  justify-items: start;
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.card.wide {
  grid-column: 1 / -1;
  justify-items: stretch;
}
.field {
  display: grid;
  gap: var(--space-2);
}
.lbl {
  font-size: var(--fs-sm);
  font-weight: 500;
}
.form {
  display: grid;
  gap: var(--space-3);
  width: 100%;
}
.state {
  margin: 0;
  font-weight: 600;
}
.bar {
  display: grid;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
.acts {
  display: flex;
  gap: var(--space-2);
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
