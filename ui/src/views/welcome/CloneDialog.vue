<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api, toApiError, type ApiError } from '@/api'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import ErrorState from '@/components/ErrorState.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import { refNameError } from '@/lib/inspect'
import ProgressBar from '@/components/ProgressBar.vue'
import TextField from '@/components/TextField.vue'
import TransferStats from '@/components/TransferStats.vue'
import { baseName } from '@/lib/persist'
import { pickFolder, useOpenRepo } from '@/shell/useOpenRepo'
import { etaOf, rateOf, useOpsStore } from '@/stores/ops'

const open = defineModel<boolean>('open', { default: false })
const ops = useOpsStore()
const { openPath } = useOpenRepo()

const url = ref('')
const dest = ref('')
const branch = ref('')
const branchError = computed(() => (branch.value ? (refNameError(branch.value) ?? undefined) : undefined))
const error = ref<ApiError | null>(null)
const opId = ref<string | null>(null)

const op = computed(() => ops.ops.find((o) => o.id === opId.value))
const running = computed(() => op.value?.status === 'running')
// Finished bars are decided; only the ones still moving are shown.
const bars = computed(() => (op.value ? [...op.value.bars.values()].filter((b) => !b.done) : []))
const urlError = computed(() => (url.value && !/^https?:\/\/[^\s/]+/.test(url.value) && !/^([a-zA-Z]:)?[\\/]/.test(url.value) ? 'Use an http(s) address or a local folder path' : undefined))

watch(open, (o) => {
  if (o && !running.value) error.value = null
})

async function browse() {
  const dir = await pickFolder('Choose where to save the clone')
  if (dir) dest.value = dir
}

async function submit() {
  if (running.value || !url.value || !dest.value || urlError.value || branchError.value) return
  error.value = null
  const target = dest.value
  let id = ''
  try {
    await ops.track(
      { kind: 'clone', label: `Clone ${baseName(url.value)}`, cancel: () => api.cancelOp(id) },
      (report, opIdent) => {
        id = opIdent
        opId.value = opIdent
        return api.clone(url.value, target, opIdent, report, branch.value.trim() || undefined)
      },
      () => `Cloned into ${target}`,
    )
  } catch (e) {
    const err = toApiError(e)
    if (err.code !== 'cancelled') error.value = err
    return
  }
  open.value = false
  await openPath(target)
}
</script>

<template>
  <Dialog v-model:open="open" title="Clone repository" description="Download a repository and all its history to this computer.">
    <form v-if="!running" class="form" @submit.prevent="submit">
      <TextField v-model="url" label="Repository URL" placeholder="https://mediagit.studio.example/hero-film" :error="urlError" mono />
      <div class="dest">
        <TextField v-model="dest" label="Save into" placeholder="D:\Projects\hero-film" mono />
        <Button @click="browse">Browse…</Button>
      </div>
      <MoreOptions data-testid="clone-more">
        <TextField v-model="branch" label="Branch to check out" hint="Leave empty for main." flag="--branch" placeholder="main" :error="branchError" mono />
      </MoreOptions>
      <ErrorState v-if="error" :code="error.code" :message="error.message" @retry="submit" />
      <div class="foot">
        <Button @click="open = false">Cancel</Button>
        <Button type="submit" variant="primary" :disabled="!url || !dest || !!urlError || !!branchError">Clone</Button>
      </div>
    </form>

    <div v-else class="run" data-testid="clone-progress">
      <p class="phase" aria-live="polite">{{ op?.phase || 'Connecting…' }}</p>
      <ul>
        <li v-for="b in bars" :key="b.id">
          <span>{{ b.msg }}</span>
          <ProgressBar :label="b.msg || 'Clone'" :value="b.len != null ? b.pos : null" :max="b.len ?? 100" />
          <TransferStats v-if="b.unit === 'bytes'" :done="b.pos" :total="b.len" :rate="rateOf(b)" :eta="etaOf(b)" />
        </li>
      </ul>
      <ProgressBar v-if="!bars.length" label="Clone" :value="null" />
      <div class="foot">
        <Button variant="danger" @click="ops.cancel(op!.id)">Cancel clone</Button>
      </div>
    </div>
  </Dialog>
</template>

<style scoped>
.form,
.run {
  display: grid;
  gap: var(--space-4);
}
.dest {
  display: grid;
  grid-template-columns: 1fr auto;
  align-items: end;
  gap: var(--space-3);
}
.foot {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-3);
}
.phase {
  margin: 0;
  font-weight: 500;
}
ul {
  display: grid;
  gap: var(--space-4);
  margin: 0;
  padding: 0;
  list-style: none;
}
li {
  display: grid;
  gap: var(--space-1);
}
</style>
