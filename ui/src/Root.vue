<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterView, useRoute } from 'vue-router'
import { api, CONTRACT_VERSION, toApiError, type ApiError } from '@/api'
import ErrorState from '@/components/ErrorState.vue'
import App from './App.vue'

const route = useRoute()
const state = ref<'checking' | 'ok' | 'blocked'>('checking')
const blocker = ref<ApiError>({ code: '', message: '' })

const PLAIN: Record<string, string> = {
  cli_not_found: 'MediaGit could not find its engine next to the app. Reinstall MediaGit to restore it.',
  contract_mismatch: 'This app and its engine were built for different versions. Reinstall MediaGit so they match.',
}

async function check() {
  state.value = 'checking'
  try {
    const info = await api.engineInfo()
    if (info.contract !== CONTRACT_VERSION) {
      throw { code: 'contract_mismatch', message: `Engine speaks contract v${info.contract}, app expects v${CONTRACT_VERSION}.` }
    }
    state.value = 'ok'
  } catch (e) {
    const err = toApiError(e)
    blocker.value = { code: err.code, message: PLAIN[err.code] ?? err.message }
    state.value = 'blocked'
  }
}
onMounted(check)
</script>

<template>
  <RouterView v-if="route.meta.bare" />
  <ErrorState v-else-if="state === 'blocked'" class="block" :code="blocker.code" :message="blocker.message" @retry="check" />
  <p v-else-if="state === 'checking'" class="boot" role="status">Starting MediaGit…</p>
  <App v-else />
</template>

<style scoped>
.block,
.boot {
  display: grid;
  place-content: center;
  height: 100%;
  margin: 0;
  color: var(--text-muted);
}
</style>
