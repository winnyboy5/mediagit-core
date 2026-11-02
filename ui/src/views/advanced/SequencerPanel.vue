<script setup lang="ts">
// Rebase and cherry-pick: choose a target, start, and if it stops on clashes decide per file, then continue.
import { computed, ref, watch } from 'vue'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import StatusBadge from '@/components/StatusBadge.vue'
import { shortOid } from '@/lib/history'
import { stepSummary, usePickStore, useRebaseStore, useVersionsStore } from '@/stores/advanced'
import { useBranchesStore } from '@/stores/branches'
import { primaryBar } from '@/stores/ops'

const props = defineProps<{ kind: 'rebase' | 'pick' }>()
const seq = props.kind === 'rebase' ? useRebaseStore() : usePickStore()
const branches = useBranchesStore()
const versions = useVersionsStore()

const isRebase = props.kind === 'rebase'
const options = computed(() =>
  isRebase
    ? branches.list.filter((b) => b.name !== branches.current).map((b) => ({ value: b.name, label: b.name }))
    : pickable.value.map((c) => ({ value: c.oid, label: `${shortOid(c.oid)} · ${c.summary || '(no message)'}` })),
)
// Cherry-pick copies from any branch: choose the branch first, then one of its versions.
const ALL = '*all'
const source = ref(ALL)
const sourceOptions = computed(() => [{ value: ALL, label: 'All branches' }, ...branches.list.map((b) => ({ value: b.name, label: b.name }))])
const pickable = computed(() => {
  if (source.value === ALL) return versions.allList
  const tip = branches.list.find((b) => b.name === source.value)?.oid
  const byOid = new Map(versions.allList.map((c) => [c.oid, c]))
  const seen = new Set<string>()
  const out = []
  const todo = tip ? [tip] : []
  while (todo.length) {
    const c = byOid.get(todo.pop()!)
    if (!c || seen.has(c.oid)) continue
    seen.add(c.oid)
    out.push(c)
    todo.push(...c.parents)
  }
  return versions.allList.filter((c) => seen.has(c.oid))
})
const target = ref('')
watch(
  options,
  (o) => {
    if (!o.some((x) => x.value === target.value)) target.value = o[0]?.value ?? ''
  },
  { immediate: true },
)

const confirmOpen = ref(false)
const abortOpen = ref(false)
const bar = computed(() => (seq.running ? primaryBar(seq.running) : undefined))
const noun = isRebase ? 'rebase' : 'copy'
const targetLabel = computed(() => options.value.find((o) => o.value === target.value)?.label ?? target.value)
</script>

<template>
  <div class="panel" :data-panel="kind" :aria-busy="seq.busy || undefined">
    <p class="muted">
      <template v-if="isRebase">Replay this branch's versions on top of another branch, as if you had started from it. This rewrites this branch's history.</template>
      <template v-else>Copy the changes of one earlier version onto the branch you are on, as a new version.</template>
    </p>

    <div v-if="seq.error" class="banner" role="alert">
      <code class="mono">{{ seq.error.code }}</code>
      <span>{{ seq.error.message }}</span>
      <Button size="sm" variant="ghost" @click="seq.error = null">Dismiss</Button>
    </div>

    <div v-if="seq.running" class="bar" role="status" :data-testid="`${kind}-progress`">
      <strong>{{ seq.running.label }}</strong>
      <ProgressBar :label="seq.running.label" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
      <span v-if="bar && bar.len != null" class="muted mono">{{ bar.pos }} / {{ bar.len }}</span>
    </div>

    <template v-if="seq.active">
      <div class="summary" :data-testid="`${kind}-summary`">
        <strong>{{ seq.clashes.length - seq.resolved }} of {{ seq.clashes.length }} files need a decision · {{ seq.resolved }} resolved</strong>
        <ProgressBar label="Clashes resolved" :value="seq.resolved" :max="seq.clashes.length || 1" />
        <span class="muted">Stopped while replaying <span class="mono">{{ shortOid(seq.conflictCommit ?? '') }}</span>. Choose a version of each file, then continue.</span>
      </div>
      <ul class="rows" aria-label="Clashing files">
        <li v-for="c in seq.clashes" :key="c.path" class="row" :data-file="c.path">
          <StatusBadge status="conflict" />
          <span class="mono path">{{ c.path }}</span>
          <span v-if="c.resolved" class="chip">{{ c.resolved === 'ours' ? 'Kept mine' : 'Kept theirs' }}</span>
          <div class="acts" role="group" :aria-label="`Choose a version of ${c.path}`">
            <Button size="sm" :variant="c.resolved === 'ours' ? 'primary' : 'secondary'" :aria-pressed="c.resolved === 'ours'" :disabled="seq.busy" @click="seq.resolve(c.path, 'ours')">Keep mine</Button>
            <Button size="sm" :variant="c.resolved === 'theirs' ? 'primary' : 'secondary'" :aria-pressed="c.resolved === 'theirs'" :disabled="seq.busy" @click="seq.resolve(c.path, 'theirs')">Keep theirs</Button>
          </div>
        </li>
      </ul>
      <div class="row-actions">
        <Button variant="primary" :disabled="!seq.canContinue || seq.busy" @click="seq.control('continue')">Continue {{ noun }}</Button>
        <Button :disabled="seq.busy" @click="seq.control('skip')">Skip this version</Button>
        <Button variant="danger" :disabled="seq.busy" @click="abortOpen = true">Cancel {{ noun }}</Button>
      </div>
    </template>

    <template v-else>
      <p v-if="seq.last" class="done" role="status" :data-testid="`${kind}-result`">{{ stepSummary(seq.last) }}.</p>
      <div class="start">
        <Select v-if="!isRebase" v-model="source" label="From branch" :options="sourceOptions" :disabled="seq.busy" />
        <Select v-model="target" :label="isRebase ? 'Rebase onto' : 'Copy this version'" :options="options" :disabled="!options.length || seq.busy" />
        <Button variant="primary" :disabled="!target || seq.busy" @click="confirmOpen = true">{{ isRebase ? 'Start rebase…' : 'Copy version' }}</Button>
      </div>
      <p v-if="!options.length" class="muted">{{ isRebase ? 'There is no other branch to rebase onto.' : 'There are no versions to copy yet.' }}</p>
      <div class="row-actions resume">
        <span class="muted">Already {{ isRebase ? 'rebasing' : 'copying' }}, from another tool?</span>
        <Button size="sm" :disabled="seq.busy" @click="seq.control('continue')">Continue {{ noun }}</Button>
        <Button size="sm" variant="ghost" :disabled="seq.busy" @click="abortOpen = true">Cancel {{ noun }}</Button>
      </div>
    </template>

    <AlertDialog
      v-model:open="confirmOpen"
      :title="isRebase ? `Rebase onto ${targetLabel}?` : 'Copy this version?'"
      :description="isRebase ? 'This branch gets new versions in place of its old ones. If it clashes you decide file by file, and you can cancel.' : `${targetLabel} is copied onto this branch as a new version. If it clashes you decide file by file.`"
      :confirm-label="isRebase ? 'Start rebase' : 'Copy version'"
      @confirm="seq.start(target)"
    />
    <AlertDialog
      v-model:open="abortOpen"
      :title="`Cancel the ${noun}?`"
      description="Everything done so far is undone and your branch goes back to how it was."
      :confirm-label="`Cancel ${noun}`"
      destructive
      @confirm="seq.control('abort')"
    />
  </div>
</template>

<style scoped>
.panel {
  position: relative;
  display: grid;
  gap: var(--space-4);
  padding: var(--space-4) 0;
}
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
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
.bar,
.summary {
  display: grid;
  gap: var(--space-2);
}
.start {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-3);
}
.start :deep(.field) {
  min-width: 320px;
}
.rows {
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.row {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--space-4);
  min-height: 48px;
  padding: var(--space-2) var(--space-5);
  border-bottom: 1px solid var(--border);
}
.row:last-child {
  border-bottom: 0;
}
.path {
  overflow-wrap: anywhere;
  font-weight: 600;
}
.chip {
  padding: 2px var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--success-soft);
  color: var(--success);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.acts,
.row-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.resume {
  gap: var(--space-3);
}
.done {
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--success-soft);
}
</style>
