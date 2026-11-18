<script setup lang="ts">
import { computed, ref } from 'vue'
import { api, toApiError, type ApiError } from '@/api'
import Button from '@/components/Button.vue'
import Dialog from '@/components/Dialog.vue'
import MoreOptions from '@/components/MoreOptions.vue'
import TextField from '@/components/TextField.vue'
import { refNameError } from '@/lib/inspect'
import { baseName } from '@/lib/persist'
import { pickFolder, useOpenRepo } from '@/shell/useOpenRepo'
import { useOpsStore } from '@/stores/ops'

// Pick a folder, ask for the first branch name, run init, open the result.
// Shared by the Welcome page and the repository switcher in the rail.
const emit = defineEmits<{ fail: [error: ApiError, retry: (() => void) | null] }>()
const ops = useOpsStore()
const { openPath } = useOpenRepo()

const open = ref(false)
const dir = ref('')
const branch = ref('')
const branchError = computed(() => (branch.value ? (refNameError(branch.value) ?? undefined) : undefined))

async function start() {
  let picked: string | null
  try {
    picked = await pickFolder('Choose a folder for the new repository')
  } catch (e) {
    emit('fail', toApiError(e), null)
    return
  }
  if (picked) {
    dir.value = picked
    branch.value = ''
    open.value = true
  }
}

function createHere() {
  if (branchError.value) return
  open.value = false
  void initIn(dir.value, branch.value.trim() || undefined)
}

async function initIn(target: string, first?: string) {
  try {
    await ops.track({ kind: 'init', label: `Create repository ${baseName(target)}` }, () => api.init(target, first), () => target)
  } catch (e) {
    emit('fail', toApiError(e), () => void initIn(target, first))
    return
  }
  const err = await openPath(target)
  if (err) emit('fail', err, () => void openPath(target))
}

defineExpose({ start })
</script>

<template>
  <Dialog v-model:open="open" title="Create repository" description="Start tracking this folder with MediaGit.">
    <form id="init-form" class="initform" @submit.prevent="createHere">
      <p class="mono initdir">{{ dir }}</p>
      <MoreOptions data-testid="init-more">
        <TextField v-model="branch" label="First branch name" hint="Leave empty for main." flag="--initial-branch" placeholder="main" :error="branchError" mono />
      </MoreOptions>
    </form>
    <template #footer>
      <Button @click="open = false">Cancel</Button>
      <Button type="submit" form="init-form" variant="primary" :disabled="!!branchError">Create</Button>
    </template>
  </Dialog>
</template>

<style scoped>
.initform {
  display: grid;
  gap: var(--space-3);
}
.initdir {
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--text-muted);
}
</style>
