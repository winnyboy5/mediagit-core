<script setup lang="ts">
import { computed } from 'vue'
import { CircleCheck, Ellipsis, PanelRight } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import DropdownMenu from '@/components/DropdownMenu.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import { primaryBar } from '@/stores/ops'
import { useChangesStore } from '@/stores/changes'
import { useRepoStore } from '@/stores/repo'
import { useShellStore } from '@/stores/shell'
import { useUndoStore } from '@/stores/undo'
import UndoDialogs from './history/UndoDialogs.vue'
import FileList from './changes/FileList.vue'
import SaveDock from './changes/SaveDock.vue'

const repo = useRepoStore()
const changes = useChangesStore()
const shell = useShellStore()
const undo = useUndoStore()
const MORE = [{ id: 'discard', label: 'Discard all unsaved changes…', danger: true }]

const loadingFirst = computed(() => !repo.status && !repo.error)
const empty = computed(() => !!repo.status && changes.counts.total === 0)
const busy = computed(() => changes.busy !== null)

// The file being staged, taken from the add op's bar message when it names a row.
const progress = computed(() => {
  const op = changes.adding
  const bar = op ? primaryBar(op) : undefined
  if (!bar?.msg) return undefined
  return { path: bar.msg, value: bar.len ? Math.round((bar.pos / bar.len) * 100) : null }
})

const MODES = [
  { value: 'tree', label: 'Tree' },
  { value: 'flat', label: 'Flat' },
]
</script>

<template>
  <section class="changes" :aria-busy="busy || undefined">
    <h2 class="sr-only">Changes</h2>
    <header class="head">
      <p class="sum" data-testid="changes-summary">
        <template v-if="changes.counts.total">{{ changes.counts.total }} {{ changes.counts.total === 1 ? 'change' : 'changes' }} since the last saved version</template>
      </p>
      <div class="tools">
        <SegmentedControl v-model="changes.mode" label="Folder view" :options="MODES" />
        <DropdownMenu :items="MORE" @select="undo.ask({ kind: 'reset', mode: 'hard', rev: 'HEAD' })">
          <IconButton label="More actions" variant="secondary" :disabled="busy || undo.busy || !changes.counts.total"><Ellipsis :size="18" /></IconButton>
        </DropdownMenu>
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

    <UndoDialogs />

    <div v-if="loadingFirst" class="pad"><SkeletonRows :count="8" /></div>

    <div v-else-if="repo.error && !repo.status" class="pad">
      <ErrorState :code="repo.error.code" :message="repo.error.message" @retry="repo.refresh()" />
    </div>

    <div v-else-if="empty" class="pad">
      <EmptyState title="No changes" description="Everything is saved. Edit a file and it shows up here.">
        <template #icon><CircleCheck :size="24" /></template>
      </EmptyState>
    </div>

    <template v-else>
      <div v-if="changes.error" class="banner" role="alert">
        <code class="mono">{{ changes.error.code }}</code>
        <span>{{ changes.error.message }}</span>
        <Button size="sm" variant="ghost" @click="changes.error = null">Dismiss</Button>
      </div>

      <div v-if="changes.selectedCount > 1" class="sel" role="group" aria-label="Selected changes">
        <span>{{ changes.selectedCount }} selected</span>
        <Button size="sm" :disabled="busy" :loading="changes.busy === 'add'" @click="changes.includeSelected()">Include selected ({{ changes.selectedCount }})</Button>
        <Button size="sm" :disabled="busy" :loading="changes.busy === 'unstage'" @click="changes.excludeSelected()">Exclude selected ({{ changes.selectedCount }})</Button>
      </div>

      <FileList
        class="list"
        :rows="changes.rows"
        :selected="changes.selected"
        group="changes"
        label="Changes"
        checks="all"
        :progress="progress"
        @select="(p, m) => changes.select(p, m)"
        @toggle="(p) => changes.toggleFolder(p)"
        @select-all="changes.selectAll()"
        @check="(p) => changes.toggleInclude(p)"
      />
      <!-- Outside the clipped list, so opening it never squeezes the file list. -->
      <MoreOptions label="Staging options" data-testid="stage-more" class="opts">
        <OptionCheck v-model="changes.addOpts.all" label="Include removed files" hint="Stage deletions together with new and changed files." flag="--all" />
        <OptionCheck v-model="changes.addOpts.update" label="Only files already tracked" hint="New files are left out." flag="--update" />
        <OptionCheck v-model="changes.addOpts.ignoreRemoval" label="Leave removed files out" flag="--ignore-removal" />
        <OptionCheck v-model="changes.addOpts.force" label="Include files the ignore list skips" flag="--force" />
        <OptionCheck v-model="changes.addOpts.dryRun" label="Preview only" hint="Show what would be staged; nothing is." flag="--dry-run" />
      </MoreOptions>
    </template>

    <SaveDock v-if="!loadingFirst && !empty && !(repo.error && !repo.status)" />
  </section>
</template>

<style scoped>
.changes {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}
.head {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-6);
}
.sum {
  margin: 0;
  color: var(--text-muted);
}
.tools {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.pad {
  flex: 1;
  padding: 0 var(--space-6);
  overflow-y: auto;
}
.list {
  min-height: 160px;
  padding: 0 var(--space-4);
}
.opts {
  flex: none;
  margin: 0 var(--space-6);
}
.sel {
  display: flex;
  flex: none;
  align-items: center;
  gap: var(--space-3);
  margin: 0 var(--space-6) var(--space-2);
  color: var(--text-muted);
}
.banner {
  display: flex;
  flex: none;
  align-items: center;
  gap: var(--space-3);
  margin: 0 var(--space-6) var(--space-2);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--danger);
  border-radius: var(--radius-control);
  background: var(--danger-soft);
}
.banner span {
  flex: 1;
}
</style>
