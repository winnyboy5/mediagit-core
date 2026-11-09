<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { onBeforeRouteLeave } from 'vue-router'
import { SETTING_COPY } from '@/lib/settingCopy'
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
import SettingRow from './settings/SettingRow.vue'
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

// Every setting the CLI knows, grouped (the form is built from `config schema`).
const GROUPS: Record<string, string> = {
  performance: 'Speed',
  storage: 'Storage',
  remotes: 'Remote servers',
}
const groupTitle = (g: string) => GROUPS[g] ?? g.charAt(0).toUpperCase() + g.slice(1)
const groups = computed(() => {
  const by = new Map<string, typeof rs.fields>()
  for (const f of rs.fields) by.set(f.group, [...(by.get(f.group) ?? []), f])
  return [...by.entries()].map(([id, fields]) => ({ id, title: groupTitle(id), fields }))
})
const labelOf = (key: string) => {
  const known = SETTING_COPY[key]
  if (known) return known.label
  const leaf = key.split('.').pop()!.replace(/_/g, ' ')
  return leaf.charAt(0).toUpperCase() + leaf.slice(1)
}
const fieldLabel = (f: (typeof rs.fields)[number]) => (f.name ? `${f.name}: ${labelOf(f.spec.key)}` : labelOf(f.spec.key))

// Unsaved rows: leaving the page would drop them, so ask first (in-app dialog, not window.confirm).
const dirtyKeys = reactive(new Set<string>())
const onDirty = (k: string, d: boolean) => void (d ? dirtyKeys.add(k) : dirtyKeys.delete(k))
const leaveOpen = ref(false)
let leaveDecision: ((ok: boolean) => void) | null = null
onBeforeRouteLeave(() => {
  if (!dirtyKeys.size) return true
  leaveOpen.value = true
  return new Promise<boolean>((res) => (leaveDecision = res))
})
watch(leaveOpen, (o) => {
  if (!o) setTimeout(() => leaveDecision && (leaveDecision(false), (leaveDecision = null)))
})
const leaveGo = () => {
  leaveDecision?.(true)
  leaveDecision = null
}
</script>

<template>
  <section class="set" :aria-busy="rs.busy || undefined">
    <header class="head">
      <div class="ttl">
        <h2 class="pt sr-only">General</h2>
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
        <p class="muted">Every setting this repository understands. An empty box uses the default. Secret values are never shown.</p>
        <ErrorState v-if="rs.error && !rs.loaded" :code="rs.error.code" :message="rs.error.message" @retry="rs.refresh()" />
        <div v-else-if="!rs.loaded"><SkeletonRows :count="3" /></div>
        <p v-else-if="!groups.length" class="muted" data-testid="no-schema">The list of settings could not be read. Close and reopen the repository to try again.</p>
        <template v-else>
          <section v-for="g in groups" :key="g.id" class="group" :aria-labelledby="`g-${g.id}`" :data-group="g.id">
            <h3 :id="`g-${g.id}`">{{ g.title }}</h3>
            <SettingRow
              v-for="f in g.fields"
              :key="f.key"
              :spec="f.spec"
              :full-key="f.key"
              :label="fieldLabel(f)"
              :description="SETTING_COPY[f.spec.key]?.description"
              :env="SETTING_COPY[f.spec.key]?.env"
              @dirty="onDirty"
              :value="f.value"
              :busy="rs.busy"
              @save="(k, v, s) => rs.setKey(k, v, s)"
              @reset="(k) => rs.unsetKey(k)"
            />
          </section>
        </template>
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
    <AlertDialog
      v-model:open="leaveOpen"
      title="Leave without saving?"
      description="Some settings are changed but not saved. If you leave now, those changes are lost."
      confirm-label="Leave and discard"
      cancel-label="Keep editing"
      destructive
      @confirm="leaveGo"
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
.pt {
  margin: 0;
  font-size: var(--fs-xl);
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
.group {
  display: grid;
  gap: var(--space-1);
  margin-top: var(--space-3);
}
.group h3 {
  margin: 0;
  font-size: var(--fs-md);
}
</style>
