<script setup lang="ts">
import { ref, watchEffect } from 'vue'
import { FolderOpen, Inbox, MoreHorizontal, Plus } from 'lucide-vue-next'
import logo from '@/assets/logo.svg'
import logo16 from '@/assets/logo-16.svg'
import Wordmark from '@/components/Wordmark.vue'
import AlertDialog from '@/components/AlertDialog.vue'
import BranchPill from '@/components/BranchPill.vue'
import Button from '@/components/Button.vue'
import Checkbox from '@/components/Checkbox.vue'
import Dialog from '@/components/Dialog.vue'
import DropdownMenu from '@/components/DropdownMenu.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import FileIcon from '@/components/FileIcon.vue'
import FileRow from '@/components/FileRow.vue'
import HeadRing from '@/components/HeadRing.vue'
import IconButton from '@/components/IconButton.vue'
import Kbd from '@/components/Kbd.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import Select from '@/components/Select.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import Tabs from '@/components/Tabs.vue'
import TagPill from '@/components/TagPill.vue'
import TextField from '@/components/TextField.vue'
import Textarea from '@/components/Textarea.vue'
import Toast from '@/components/Toast.vue'
import Tooltip from '@/components/Tooltip.vue'
import TransferStats from '@/components/TransferStats.vue'
import { useToast } from '@/components/useToast'

const theme = ref<'system' | 'light' | 'dark'>('system')
const density = ref<'comfortable' | 'compact'>('comfortable')
watchEffect(() => {
  const el = document.documentElement
  if (theme.value === 'system') el.removeAttribute('data-theme')
  else el.setAttribute('data-theme', theme.value)
  if (density.value === 'compact') el.setAttribute('data-density', 'compact')
  else el.removeAttribute('data-density')
})

const toast = useToast()
const name = ref('')
const msg = ref('')
const check = ref<boolean | 'indeterminate'>(true)
const mixed = ref<boolean | 'indeterminate'>('indeterminate')
const branch = ref('main')
const tab = ref('files')
const view = ref('tree')
const dlg = ref(false)
const alert = ref(false)

const STATUSES = ['added', 'modified', 'deleted', 'locked', 'untracked', 'conflict'] as const
const PATHS = [
  'shots/sh010/plate_v003.exr',
  'footage/day1/A001_C004.mov',
  'audio/mix/final_master.wav',
  'models/hero_rig.blend',
  'design/key_art.psd',
  'vdb/smoke_0120.vdb',
  'docs/brief.pdf',
  'misc/notes.bin',
]
</script>

<template>
  <main class="kit">
    <header class="top">
      <span class="brand"><img :src="logo16" alt="" width="32" height="32" />MediaGit</span>
      <h1>Component kit</h1>
      <div class="toggles">
        <SegmentedControl
          v-model="theme"
          label="Theme"
          :options="[
            { value: 'system', label: 'System' },
            { value: 'light', label: 'Light' },
            { value: 'dark', label: 'Dark' },
          ]"
        />
        <SegmentedControl
          v-model="density"
          label="Density"
          :options="[
            { value: 'comfortable', label: 'Comfortable' },
            { value: 'compact', label: 'Compact' },
          ]"
        />
      </div>
    </header>

    <section data-section="brand">
      <h2>Brand and type</h2>
      <div class="panel row-wrap">
        <img :src="logo" alt="MediaGit mark" width="64" height="64" />
        <img :src="logo16" alt="" width="16" height="16" />
        <img :src="logo16" alt="" width="32" height="32" />
        <Wordmark :height="32" />
        <div>
          <p class="t-xl">Save version, get latest, lock files</p>
          <p>Body text at 14 px. Plain words first, version-control term second.</p>
          <p class="muted">Muted helper text, 14 px. <span class="mono">a3f9c2e1 · 1.24 GB · 00:01:12:08</span></p>
        </div>
      </div>
    </section>

    <section data-section="palette">
      <h2>Palette</h2>
      <div class="panel swatches">
        <span v-for="t in ['bg', 'surface', 'surface-2', 'surface-3', 'border', 'text', 'text-muted', 'accent', 'accent-soft']" :key="t" class="sw">
          <i :style="{ background: `var(--${t})` }" />{{ t }}
        </span>
        <span v-for="k in ['image', 'video', 'audio', 'model3d', 'document', 'vfx', 'other']" :key="k" class="sw">
          <i :style="{ background: `var(--media-${k})` }" />{{ k }}
        </span>
        <span v-for="n in 8" :key="n" class="sw"><i :style="{ background: `var(--lane-${n - 1})` }" />lane-{{ n - 1 }}</span>
      </div>
    </section>

    <section data-section="buttons">
      <h2>Buttons</h2>
      <div class="panel row-wrap">
        <Button variant="primary">Save version</Button>
        <Button variant="secondary">Get latest</Button>
        <Button variant="ghost">Cancel</Button>
        <Button variant="danger">Force unlock</Button>
        <Button variant="primary" size="sm">Small</Button>
        <Button variant="primary" loading>Saving</Button>
        <Button variant="secondary" disabled>Disabled</Button>
        <IconButton label="Add file"><Plus :size="18" /></IconButton>
        <IconButton label="Open folder" variant="secondary" size="sm"><FolderOpen :size="16" /></IconButton>
        <IconButton label="Toggle sidebar" pressed><Inbox :size="18" /></IconButton>
        <Tooltip text="Open the repository folder"><IconButton label="Open folder tooltip"><FolderOpen :size="18" /></IconButton></Tooltip>
        <span>Shortcut <Kbd>Ctrl</Kbd> + <Kbd>Enter</Kbd></span>
      </div>
    </section>

    <section data-section="forms">
      <h2>Form fields</h2>
      <div class="panel grid2">
        <TextField v-model="name" label="Repository name" placeholder="my-project" hint="Letters, numbers and dashes." />
        <TextField label="Remote URL" mono model-value="https://" error="Enter a full URL including the path." />
        <Textarea v-model="msg" label="Version message" placeholder="What changed?" :rows="3" />
        <Select
          v-model="branch"
          label="Branch"
          :options="[
            { value: 'main', label: 'main' },
            { value: 'feature/grade', label: 'feature/grade' },
            { value: 'release', label: 'release' },
          ]"
        />
        <div class="stack">
          <Checkbox v-model="check" label="Stage all changes" />
          <Checkbox v-model="mixed" label="Some files selected" />
          <Checkbox :model-value="false" label="Disabled option" disabled />
        </div>
      </div>
    </section>

    <section data-section="navigation">
      <h2>Tabs and segmented control</h2>
      <div class="panel">
        <SegmentedControl
          v-model="view"
          label="Layout"
          :options="[
            { value: 'tree', label: 'Tree' },
            { value: 'flat', label: 'Flat' },
          ]"
        />
        <Tabs
          v-model="tab"
          label="Inspector sections"
          :items="[
            { value: 'files', label: 'Files' },
            { value: 'history', label: 'History' },
            { value: 'locks', label: 'Locks' },
          ]"
        >
          <template #default="{ value }"><p class="muted">Panel: {{ value }}</p></template>
        </Tabs>
      </div>
    </section>

    <section data-section="overlays">
      <h2>Overlays and toast</h2>
      <div class="panel row-wrap">
        <Dialog v-model:open="dlg" title="Clone repository" description="Copy a MediaGit repository to this computer.">
          <template #trigger><Button>Open dialog</Button></template>
          <TextField label="Remote URL" placeholder="https://studio.example/repo" />
          <template #footer>
            <Button variant="ghost" @click="dlg = false">Cancel</Button>
            <Button variant="primary" @click="dlg = false">Clone</Button>
          </template>
        </Dialog>
        <AlertDialog
          v-model:open="alert"
          destructive
          title="Force unlock this file?"
          description="Maya Chen holds this lock. Unlocking it may discard her unsaved changes."
          confirm-label="Force unlock"
        >
          <template #trigger><Button variant="danger">Open alert</Button></template>
        </AlertDialog>
        <DropdownMenu
          :items="[
            { id: 'stage', label: 'Stage file' },
            { id: 'lock', label: 'Lock file' },
            { id: 'discard', label: 'Discard changes', danger: true, separated: true },
          ]"
        >
          <IconButton label="Row actions" variant="secondary"><MoreHorizontal :size="18" /></IconButton>
        </DropdownMenu>
        <Button @click="toast.push({ kind: 'success', title: 'Pushed 14 files', description: '3.4 GB in 1m 12s' })">Success toast</Button>
        <Button @click="toast.push({ kind: 'error', title: 'Push failed', description: 'Network dropped. Your files are safe; try again.' })">Error toast</Button>
        <Button @click="toast.push({ title: 'Fetched latest' })">Info toast</Button>
      </div>
    </section>

    <section data-section="progress">
      <h2>Progress</h2>
      <div class="panel stack">
        <div>
          <ProgressBar label="Upload" :value="42" />
          <TransferStats :done="1.4e9" :total="3.4e9" :rate="38e6" :eta="53" />
        </div>
        <div>
          <ProgressBar label="Chunking" />
          <TransferStats :done="null" :total="null" />
        </div>
        <ProgressBar label="Small determinate" size="sm" :value="70" />
        <ProgressBar label="Small indeterminate" size="sm" />
      </div>
    </section>

    <section data-section="status">
      <h2>Status, branches and tags</h2>
      <div class="panel row-wrap">
        <StatusBadge v-for="s in STATUSES" :key="s" :status="s" />
        <BranchPill name="main" current :lane="1" />
        <BranchPill name="feature/grade" :lane="2" />
        <TagPill name="v0.7.0-rc.1" />
        <HeadRing />
      </div>
    </section>

    <section data-section="files">
      <h2>Files</h2>
      <div class="panel list">
        <div class="row-wrap icons"><FileIcon v-for="p in PATHS" :key="p" :path="p" /></div>
        <FileRow path="shots/sh010/plate_v003.exr" :size="412e6" status="modified" selected />
        <FileRow path="footage/day1/A001_C004.mov" :size="3.2e9" status="added" :progress="64" />
        <FileRow path="audio/mix/final_master.wav" :size="88e6" status="locked" :progress="null" />
        <FileRow path="models/hero_rig.blend" :size="52e6" status="conflict" />
        <FileRow path="design/key_art.psd" :size="210e6" status="deleted" />
        <FileRow path="docs/brief.pdf" :size="1.2e6" status="untracked" />
        <FileRow path="misc/notes.bin" :size="3400" />
      </div>
    </section>

    <section data-section="states">
      <h2>Loading, empty, error</h2>
      <div class="panel grid3">
        <SkeletonRows :count="4" />
        <EmptyState title="No changes" description="Everything is saved. Edit a file and it will show up here.">
          <template #icon><Inbox :size="26" /></template>
          <Button variant="secondary">Open folder</Button>
        </EmptyState>
        <ErrorState code="E_REMOTE_UNREACHABLE" message="Could not reach the remote. Check your connection and try again." />
      </div>
    </section>

    <Toast />
  </main>
</template>

<style scoped>
.kit {
  max-width: 1120px;
  margin: 0 auto;
  padding: var(--space-8) var(--space-6) var(--space-12);
}
.top {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-6);
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--fs-xl);
  font-weight: 600;
}
h1 {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-lg);
  font-weight: 500;
}
.toggles {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
  margin-left: auto;
}
section {
  margin-top: var(--space-8);
}
h2 {
  margin: 0 0 var(--space-3);
  font-size: var(--fs-lg);
  font-weight: 600;
}
.panel {
  padding: var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  box-shadow: var(--shadow-1);
}
.row-wrap {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}
.stack {
  display: grid;
  align-content: start;
  gap: var(--space-4);
}
.grid2 {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-6);
}
.grid3 {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: var(--space-6);
}
.list {
  padding: var(--space-3);
}
.icons {
  padding: var(--space-3) var(--space-4) var(--space-4);
}
.swatches {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3) var(--space-6);
}
.sw {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--fs-sm);
}
.sw i {
  width: 24px;
  height: 24px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-control);
}
p {
  margin: 0 0 var(--space-2);
}
.t-xl {
  font-size: var(--fs-xl);
  font-weight: 600;
}
.muted {
  color: var(--text-muted);
}
</style>
