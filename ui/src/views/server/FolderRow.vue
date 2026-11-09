<script setup lang="ts">
import { computed } from 'vue'
import { FolderOpen } from 'lucide-vue-next'
import { toApiError } from '@/api'
import Button from '@/components/Button.vue'
import { pickFolder } from '@/shell/useOpenRepo'
import { useServerStore } from '@/stores/server'

const server = useServerStore()
const path = computed(() => server.local?.data_dir ?? '')
const running = computed(() => server.local?.state === 'running' || server.local?.state === 'elsewhere')

async function choose() {
  try {
    const dir = await pickFolder('Choose the server folder')
    if (dir) await server.setFolder(dir)
  } catch (e) {
    server.folderError = toApiError(e)
  }
}
</script>

<template>
  <div class="folder" data-testid="folder-row">
    <p class="label">Server folder</p>
    <p class="mono path" data-testid="folder-path">{{ path }}</p>
    <div class="btns">
      <Button size="sm" :disabled="running || server.busy" @click="choose"><FolderOpen :size="14" aria-hidden="true" /> Choose folder…</Button>
      <Button size="sm" :disabled="running || server.busy" @click="server.setFolder()">Use default</Button>
    </div>
    <p v-if="running" class="note">Stop the server to change its folder.</p>
    <p v-if="server.folderError" class="err" role="alert" data-testid="folder-error">{{ server.folderError.message }}</p>
    <p class="note">
      A server and your own projects can live on the same computer, but not inside each other. Pick a folder that is empty or already holds a server.
    </p>
  </div>
</template>

<style scoped>
.folder {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}
.label {
  margin: 0;
  font-weight: 600;
}
.path {
  margin: 0;
  font-size: var(--fs-sm);
  overflow-wrap: anywhere;
}
.btns {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.note {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.err {
  margin: 0;
  color: var(--danger);
}
</style>
