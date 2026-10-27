<script setup lang="ts">
// Preview of the selected file: image, video or audio via the asset protocol,
// icon + note for everything else. A file inside a version is extracted first.
import { computed, ref, watch, watchEffect } from 'vue'
import { api } from '@/api'
import Button from '@/components/Button.vue'
import FileIcon from '@/components/FileIcon.vue'
import ProgressBar from '@/components/ProgressBar.vue'
import TransferStats from '@/components/TransferStats.vue'
import { extOf } from '@/components/media'
import { EXTRACT_PROGRESS_MIN, previewKind, type InspectFile } from '@/lib/inspect'
import { fileKey, useInspectorStore } from '@/stores/inspector'

const props = defineProps<{ file: InspectFile }>()
const inspector = useInspectorStore()

const kind = computed(() => previewKind(props.file.path))
const name = computed(() => props.file.path.split('/').pop() ?? props.file.path)
const gone = computed(() => props.file.deleted || props.file.status === 'deleted')
const wantsBlob = computed(() => kind.value !== 'none' && !gone.value)

watchEffect(() => {
  if (wantsBlob.value) inspector.ensureExtraction(props.file)
})
const x = computed(() => inspector.extractions.get(fileKey(props.file)))
const src = computed(() => (x.value?.status === 'ready' && x.value.path ? api.fileSrc(x.value.path) : undefined))
const big = computed(() => (props.file.size ?? 0) > EXTRACT_PROGRESS_MIN)

// A file the WebView cannot decode reports it on the element.
const broken = ref(false)
watch(() => [props.file.path, props.file.rev, src.value], () => (broken.value = false))
</script>

<template>
  <div class="prev" data-testid="preview">
    <template v-if="gone">
      <FileIcon :path="file.path" />
      <p>This file is deleted here, so there is nothing to preview.</p>
    </template>

    <template v-else-if="kind === 'none'">
      <FileIcon :path="file.path" />
      <p>No preview for .{{ extOf(file.path) || 'this' }} files. Details are below.</p>
    </template>

    <div v-else-if="x?.status === 'loading' || !x" class="load" role="status">
      <template v-if="big">
        <p class="cap">Getting {{ name }} from history</p>
        <ProgressBar :label="`Getting ${name}`" :value="x && x.len ? x.pos : null" :max="x?.len ?? 100" />
        <TransferStats :done="x?.pos ?? 0" :total="x?.len" />
      </template>
      <template v-else>
        <span class="shim" aria-hidden="true" />
        <span class="sr-only">Loading preview</span>
      </template>
    </div>

    <div v-else-if="x.status === 'error'" class="err" role="alert">
      <p>Could not get this file: {{ x.error?.message }}</p>
      <Button size="sm" @click="inspector.retryExtraction(file)">Retry</Button>
    </div>

    <p v-else-if="broken" class="note">This computer cannot display {{ name }}. Details are below.</p>

    <template v-else-if="src">
      <img v-if="kind === 'image'" :src="src" :alt="`Preview of ${name}`" @error="broken = true" />
      <video v-else-if="kind === 'video'" :src="src" controls preload="metadata" :aria-label="`Preview of ${name}`" @error="broken = true">
        <track kind="captions" label="No captions available" />
      </video>
      <div v-else class="aud">
        <FileIcon :path="file.path" />
        <audio :src="src" controls preload="metadata" :aria-label="`Preview of ${name}`" @error="broken = true" />
      </div>
    </template>
  </div>
</template>

<style scoped>
.prev {
  display: grid;
  place-items: center;
  gap: var(--space-3);
  min-height: 160px;
  padding: var(--space-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface-2);
  color: var(--text-muted);
  text-align: center;
}
p {
  margin: 0;
  font-size: var(--fs-sm);
}
img,
video {
  display: block;
  width: 100%;
  max-height: 260px;
  border-radius: var(--radius-control);
  object-fit: contain;
}
audio {
  width: 100%;
}
.aud {
  display: grid;
  gap: var(--space-3);
  justify-items: center;
  width: 100%;
}
.load,
.err {
  display: grid;
  gap: var(--space-2);
  justify-items: center;
  width: 100%;
}
.cap {
  color: var(--text);
}
.shim {
  width: 100%;
  height: 120px;
  border-radius: var(--radius-control);
  background: linear-gradient(90deg, var(--surface-3), var(--surface-2), var(--surface-3));
  background-size: 200% 100%;
  animation: shim 1.2s linear infinite;
}
@keyframes shim {
  to {
    background-position: -200% 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  .shim {
    animation: none;
  }
}
.err {
  color: var(--danger);
}
</style>
