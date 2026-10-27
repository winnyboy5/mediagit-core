<script setup lang="ts">
import { computed } from 'vue'
import { CircleCheck, PanelRight, RefreshCw } from 'lucide-vue-next'
import Button from '@/components/Button.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorState from '@/components/ErrorState.vue'
import IconButton from '@/components/IconButton.vue'
import Kbd from '@/components/Kbd.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import SkeletonRows from '@/components/SkeletonRows.vue'
import Textarea from '@/components/Textarea.vue'
import Tooltip from '@/components/Tooltip.vue'
import { primaryBar } from '@/stores/ops'
import { useChangesStore } from '@/stores/changes'
import { useRepoStore } from '@/stores/repo'
import { useShellStore } from '@/stores/shell'
import FileList from './changes/FileList.vue'

const repo = useRepoStore()
const changes = useChangesStore()
const shell = useShellStore()

const loadingFirst = computed(() => !repo.status && !repo.error)
const empty = computed(() => !!repo.status && changes.counts.total === 0)
const busy = computed(() => changes.busy !== null)

const selectedUnstaged = computed(() => [...changes.selected].filter((k) => k.startsWith('unstaged:')).length)
const canCommit = computed(() => changes.counts.staged > 0 && changes.message.trim().length > 0 && !busy.value)
const commitHint = computed(() =>
  changes.counts.staged === 0 ? 'Stage files first, then save a version.' : changes.message.trim() ? '' : 'Describe what changed to save.',
)

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
    <header class="head">
      <div class="ttl">
        <h1>Changes</h1>
        <p>what changed since the last saved version</p>
      </div>
      <div class="tools">
        <SegmentedControl v-model="changes.mode" label="Folder view" :options="MODES" />
        <IconButton label="Refresh changes" variant="secondary" :disabled="repo.loading" @click="repo.refresh()">
          <RefreshCw :size="18" />
        </IconButton>
        <IconButton
          :label="shell.inspectorVisible ? 'Hide details panel' : 'Show details panel'"
          variant="secondary"
          :pressed="shell.inspectorVisible"
          @click="shell.toggleInspector()"
        >
          <PanelRight :size="18" />
        </IconButton>
      </div>
    </header>

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

      <div class="panels">
        <section v-if="changes.counts.staged" class="panel staged" aria-labelledby="h-staged">
          <header>
            <h2 id="h-staged">Staged · {{ changes.counts.staged }} {{ changes.counts.staged === 1 ? 'file' : 'files' }}</h2>
            <Tooltip text="Unstaging arrives in a later release">
              <Button size="sm" aria-disabled="true" class="soon" @click.prevent>Unstage all</Button>
            </Tooltip>
          </header>
          <FileList
            :rows="changes.rows.staged"
            :selected="changes.selected"
            group="staged"
            label="Staged files"
            @select="(p, m) => changes.select('staged', p, m)"
            @toggle="(p) => changes.toggleFolder('staged', p)"
            @select-all="changes.selectAll('staged')"
          />
        </section>

        <section v-if="changes.counts.unstaged" class="panel unstaged" aria-labelledby="h-unstaged">
          <header>
            <h2 id="h-unstaged">Not staged · {{ changes.counts.unstaged }} {{ changes.counts.unstaged === 1 ? 'file' : 'files' }}</h2>
            <div class="acts">
              <Button v-if="selectedUnstaged" size="sm" :disabled="busy" :loading="changes.busy === 'add'" @click="changes.stageSelected()">
                Stage selected ({{ selectedUnstaged }})
              </Button>
              <Button size="sm" :disabled="busy" @click="changes.stageAll()">Stage all</Button>
            </div>
          </header>
          <FileList
            :rows="changes.rows.unstaged"
            :selected="changes.selected"
            group="unstaged"
            label="Files not staged"
            :progress="progress"
            @select="(p, m) => changes.select('unstaged', p, m)"
            @toggle="(p) => changes.toggleFolder('unstaged', p)"
            @select-all="changes.selectAll('unstaged')"
          />
        </section>
      </div>

      <form class="commit" @submit.prevent="changes.commit()">
        <Textarea
          v-model="changes.message"
          label="Message"
          placeholder="What changed? e.g. Grade pass on shot_010"
          :rows="3"
          :disabled="busy"
          @keydown.ctrl.enter.prevent="changes.commit()"
        />
        <div class="row">
          <p class="hint"><template v-if="commitHint">{{ commitHint }}</template><template v-else><Kbd>Ctrl</Kbd> + <Kbd>Enter</Kbd> to save</template></p>
          <Button type="submit" variant="primary" :disabled="!canCommit" :loading="changes.busy === 'commit'">Save version (commit)</Button>
        </div>
      </form>
    </template>
  </section>
</template>

<style scoped>
.changes {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
  padding: var(--space-5) var(--space-6);
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
h1 {
  margin: 0;
  font-size: var(--fs-2xl);
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
.pad {
  overflow-y: auto;
}
.panels {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-4);
  min-height: 0;
}
.panel {
  display: flex;
  flex: 1 1 0;
  flex-direction: column;
  min-height: 120px;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  overflow: hidden;
}
.panel.staged {
  flex-grow: 0.6;
}
.panel > header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-2) var(--space-4);
  border-bottom: 1px solid var(--border);
  background: var(--surface-2);
}
h2 {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.acts {
  display: flex;
  gap: var(--space-2);
}
.soon {
  opacity: 0.55;
  cursor: not-allowed;
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
.commit {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}
.hint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
