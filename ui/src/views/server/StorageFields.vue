<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { CircleCheck, CircleX, FolderOpen, PlugZap } from 'lucide-vue-next'
import type { StorageKind, StorageOpts } from '@/api'
import Button from '@/components/Button.vue'
import SegmentedControl from '@/components/SegmentedControl.vue'
import TextField from '@/components/TextField.vue'
import { useServerStore } from '@/stores/server'

/** Where the server keeps its data. The parent owns the object; secrets are never copied anywhere else. */
const s = defineModel<StorageOpts>({ required: true })
const server = useServerStore()

const KINDS = [
  { value: 'filesystem', label: 'Local disk' },
  { value: 's3', label: 'Amazon S3 or MinIO' },
  { value: 'gcs', label: 'Google Cloud' },
  { value: 'azure', label: 'Azure Blob' },
]
const kind = computed({
  get: () => s.value.kind,
  set: (k: string) => (s.value = { kind: k as StorageKind }),
})
const field = (k: keyof StorageOpts) =>
  computed({
    get: () => (s.value[k] as string | undefined) ?? '',
    set: (v: string) => ((s.value as unknown as Record<string, unknown>)[k] = v),
  })
const bucket = field('bucket')
const region = field('region')
const endpoint = field('endpoint')
const prefix = field('prefix')
const akid = field('access_key_id')
const secret = field('secret_access_key')
const project = field('project')
const keyFile = field('credentials_path')
const account = field('account')
const container = field('container')
const accountKey = field('account_key')
const connStr = field('connection_string')

async function pickKeyFile() {
  try {
    const f = await openDialog({ multiple: false, title: 'Service account key file', filters: [{ name: 'JSON', extensions: ['json'] }] })
    if (typeof f === 'string') keyFile.value = f
  } catch {
    /* cancelled or unavailable: the path can still be typed */
  }
}

// Test connection -----------------------------------------------------------------
const testing = ref(false)
const result = ref<{ ok: boolean; error?: string } | null>(null)
watch(s, () => (result.value = null), { deep: true })
async function test() {
  testing.value = true
  result.value = null
  const r = await server.testStorage({ ...s.value })
  testing.value = false
  result.value = r
}
</script>

<template>
  <fieldset class="storage" data-testid="storage-fields">
    <legend>Where should the server keep its files?</legend>
    <SegmentedControl v-model="kind" :options="KINDS" label="Storage" />
    <p v-if="kind === 'filesystem'" class="hint">On this computer, inside the server folder. Nothing else to set up.</p>

    <template v-else>
      <div v-if="kind === 's3'" class="grid">
        <TextField v-model="bucket" label="Bucket" />
        <TextField v-model="region" label="Region" placeholder="us-east-1" />
        <TextField v-model="endpoint" label="Endpoint" hint="Only for MinIO or other S3-compatible storage." placeholder="http://localhost:9000" />
        <TextField v-model="prefix" label="Folder inside the bucket" hint="Optional." />
        <TextField v-model="akid" label="Access key ID" />
        <TextField v-model="secret" label="Secret key" type="password" />
      </div>
      <div v-else-if="kind === 'gcs'" class="grid">
        <TextField v-model="bucket" label="Bucket" />
        <TextField v-model="project" label="Project" />
        <TextField v-model="prefix" label="Folder inside the bucket" hint="Optional." />
        <div class="pick">
          <TextField v-model="keyFile" label="Service account key file" mono hint="Leave empty to use the Google sign-in already on this computer." />
          <Button size="sm" @click="pickKeyFile"><FolderOpen :size="14" aria-hidden="true" /> Choose file…</Button>
        </div>
      </div>
      <div v-else class="grid">
        <p class="hint wide">Azure support is in preview.</p>
        <TextField v-model="account" label="Storage account" />
        <TextField v-model="container" label="Container" />
        <TextField v-model="prefix" label="Folder inside the container" hint="Optional." />
        <TextField v-model="accountKey" label="Account key" type="password" />
        <TextField v-model="connStr" label="Or a connection string" type="password" hint="Use this instead of the account key." />
      </div>

      <div class="test">
        <Button :loading="testing" @click="test"><PlugZap :size="16" aria-hidden="true" /> Test connection</Button>
        <p v-if="result?.ok" class="ok" role="status" data-testid="storage-ok"><CircleCheck :size="16" aria-hidden="true" /> The server can read and write there.</p>
        <p v-else-if="result" class="bad" role="alert" data-testid="storage-bad"><CircleX :size="16" aria-hidden="true" /> {{ result.error || 'The connection failed.' }}</p>
        <p v-else-if="!testing" class="hint">Testing is optional, but a wrong setting is easier to fix now than later.</p>
      </div>
    </template>
  </fieldset>
</template>

<style scoped>
.storage {
  display: grid;
  gap: var(--space-3);
  margin: 0;
  padding: 0;
  border: 0;
  min-width: 0;
}
legend {
  padding: 0;
  margin-bottom: var(--space-2);
  font-weight: 600;
}
.grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
}
.wide {
  grid-column: 1 / -1;
}
.pick {
  grid-column: 1 / -1;
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}
.pick :deep(.field) {
  width: 100%;
}
.hint {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.test {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}
.ok,
.bad {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin: 0;
  font-size: var(--fs-sm);
}
.ok {
  color: var(--success);
}
.bad {
  color: var(--danger);
}
</style>
