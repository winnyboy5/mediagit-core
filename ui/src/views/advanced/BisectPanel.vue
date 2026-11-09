<script setup lang="ts">
// A bisect wizard: pick a bad and a good version, then say good, bad or skip until the first bad one is found.
import { computed, ref, watch } from 'vue'
import { Bug } from 'lucide-vue-next'
import AlertDialog from '@/components/AlertDialog.vue'
import Button from '@/components/Button.vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import MoreOptions from '@/components/MoreOptions.vue'
import OptionCheck from '@/components/OptionCheck.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import Select from '@/components/Select.vue'
import { shortOid } from '@/lib/history'
import { useBisectStore, useVersionsStore } from '@/stores/advanced'
import { primaryBar } from '@/stores/ops'

const bisect = useBisectStore()
const versions = useVersionsStore()

const options = computed(() => versions.list.map((c) => ({ value: c.oid, label: `${shortOid(c.oid)} · ${c.summary || '(no message)'}` })))
const good = ref('')
watch(
  options,
  (o) => {
    // The oldest loaded version is the likeliest "known good".
    if (!o.some((x) => x.value === good.value)) good.value = o[o.length - 1]?.value ?? ''
  },
  { immediate: true },
)

const r = computed(() => bisect.report)
const bar = computed(() => (bisect.running ? primaryBar(bisect.running) : undefined))
const resetOpen = ref(false)
const restart = ref(false)
const restartOpen = ref(false)
const moveOpen = ref(false)
const other = ref('')
watch(options, (o) => !o.some((x) => x.value === other.value) && (other.value = o[0]?.value ?? ''), { immediate: true })
// Throwing away a running bisect needs a yes; a plain start does not.
function begin() {
  if (restart.value === true) restartOpen.value = true
  else void bisect.start('HEAD', good.value)
}
async function replay() {
  const file = await openDialog({ multiple: false, directory: false, title: 'Choose a bisect log' })
  if (typeof file === 'string') void bisect.replay(file)
}
const name = (oid: string | null) => (oid ? `${shortOid(oid)}${versions.summaryOf(oid) ? ` · ${versions.summaryOf(oid)}` : ''}` : '')
</script>

<template>
  <div class="panel" data-panel="bisect" :aria-busy="bisect.busy || undefined">
    <p class="muted">Find which version broke something. MediaGit checks out a version in the middle; you test it and say whether it is good or bad, and it narrows the range each time.</p>

    <div v-if="bisect.error" class="banner" role="alert">
      <code class="mono">{{ bisect.error.code }}</code>
      <span>{{ bisect.error.message }}</span>
      <Button size="sm" variant="ghost" @click="bisect.error = null">Dismiss</Button>
    </div>

    <div v-if="bisect.running" class="bar" role="status" data-testid="bisect-progress">
      <strong>{{ bisect.running.label }}</strong>
      <ProgressBar :label="bisect.running.label" :value="bar && bar.len != null ? bar.pos : null" :max="bar?.len ?? 100" />
    </div>

    <div v-if="!bisect.active" class="start">
      <p v-if="r?.status === 'reset'" class="done" role="status">Bisect ended. Your files are back where they were.</p>
      <Select v-model="good" label="A version that was working" :options="options" :disabled="!options.length || bisect.busy" />
      <Button variant="primary" :disabled="!good || bisect.busy" @click="begin"><Bug :size="16" aria-hidden="true" /> Start bisect</Button>
      <p class="muted">The latest version is taken as the bad one.</p>
      <MoreOptions>
        <OptionCheck v-model="restart" danger label="Throw away a bisect that is already running" flag="--reset" hint="Everything marked so far is forgotten." :disabled="bisect.busy" />
        <div>
          <Button :disabled="bisect.busy" @click="replay">Replay a saved bisect log…</Button>
          <p class="muted">Re-applies the good and bad marks recorded in a log file. <code class="mono">bisect replay</code></p>
        </div>
      </MoreOptions>
    </div>

    <div v-else-if="r" class="wizard" data-testid="bisect-state">
      <template v-if="r.status === 'complete'">
        <p class="found" role="status"><strong>The first bad version is {{ name(r.first_bad) }}.</strong></p>
      </template>
      <template v-else-if="r.status === 'waiting_for_good'">
        <p role="status"><strong>Now choose a version that worked.</strong> <span class="muted">Bad so far: {{ name(r.bad) }}</span></p>
      </template>
      <template v-else>
        <p role="status">
          <strong>Test this version: {{ name(r.current) }}</strong><br />
          <span class="muted">About {{ r.remaining_steps ?? '?' }} {{ r.remaining_steps === 1 ? 'step' : 'steps' }} left.</span>
        </p>
        <div class="verdicts" role="group" aria-label="Does it work?">
          <Button variant="primary" :disabled="bisect.busy" @click="bisect.mark('good')">It works (good)</Button>
          <Button :disabled="bisect.busy" @click="bisect.mark('bad')">It is broken (bad)</Button>
          <Button :disabled="bisect.busy" @click="bisect.mark('skip')">Cannot tell (skip)</Button>
        </div>
      </template>
      <dl class="kv">
        <dt>Known bad</dt><dd class="mono">{{ r.bad ? shortOid(r.bad) : '–' }}</dd>
        <dt>Known good</dt><dd class="mono">{{ r.good.map(shortOid).join(', ') || '–' }}</dd>
        <dt>Skipped</dt><dd class="mono">{{ r.skipped.map(shortOid).join(', ') || '–' }}</dd>
      </dl>
      <Button :disabled="bisect.busy" @click="resetOpen = true">End bisect…</Button>
      <MoreOptions>
        <Select v-model="other" label="A different version" :options="options" :disabled="bisect.busy" />
        <div class="verdicts" role="group" aria-label="Mark that version">
          <Button :disabled="!other || bisect.busy" @click="bisect.mark('good', other)">Mark it good</Button>
          <Button :disabled="!other || bisect.busy" @click="bisect.mark('bad', other)">Mark it bad</Button>
          <Button :disabled="!other || bisect.busy" @click="bisect.mark('skip', other)">Skip it</Button>
        </div>
        <Button variant="danger" :disabled="!other || bisect.busy" @click="moveOpen = true">End bisect and move my branch to it…</Button>
      </MoreOptions>
    </div>

    <AlertDialog
      v-model:open="restartOpen"
      title="Throw away the running bisect?"
      description="Everything you marked in it is forgotten and a new one starts."
      confirm-label="Start a new bisect"
      destructive
      @confirm="bisect.start('HEAD', good, true)"
    />
    <AlertDialog
      v-model:open="moveOpen"
      title="Move your branch to that version?"
      :description="`The bisect ends and your branch, and your files, jump to ${name(other)}. Versions after it leave the branch.`"
      confirm-label="End and move branch"
      destructive
      @confirm="bisect.reset(other)"
    />

    <AlertDialog
      v-model:open="resetOpen"
      title="End the bisect?"
      description="Your files go back to the version you started from. What you have marked so far is forgotten."
      confirm-label="End bisect"
      @confirm="bisect.reset()"
    />
  </div>
</template>

<style scoped>
.panel {
  display: grid;
  gap: var(--space-4);
  padding: var(--space-4) 0;
}
.muted {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
p {
  margin: 0;
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
.bar {
  display: grid;
  gap: var(--space-2);
}
.start,
.wizard {
  display: grid;
  gap: var(--space-3);
  justify-items: start;
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
}
.start :deep(.field) {
  min-width: 320px;
}
.done,
.found {
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-control);
  background: var(--success-soft);
}
.verdicts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.kv {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  margin: 0;
  font-size: var(--fs-sm);
}
.kv dt {
  color: var(--text-muted);
}
.kv dd {
  margin: 0;
  overflow-wrap: anywhere;
}
</style>
