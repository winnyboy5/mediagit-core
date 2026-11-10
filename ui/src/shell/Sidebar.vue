<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { ChevronsUpDown, GitBranch } from 'lucide-vue-next'
import logo from '@/assets/logo-16.svg'
import DropdownMenu, { type MenuItem } from '@/components/DropdownMenu.vue'
import { timeAgo } from '@/lib/persist'
import { useChangesStore } from '@/stores/changes'
import { useLocksStore } from '@/stores/locks'
import { useRecentStore } from '@/stores/recent'
import { useRepoStore } from '@/stores/repo'
import { useOpsStore } from '@/stores/ops'
import { useShellStore } from '@/stores/shell'
import { useSyncStore } from '@/stores/sync'
import { useUnlockStore } from '@/stores/unlock'
import { useOpenRepo } from './useOpenRepo'
import { HUBS, WORK_HUBS, hubHome, inHub, type Hub } from './hubs'

const repo = useRepoStore()
const recent = useRecentStore()
const locks = useLocksStore()
const changes = useChangesStore()
const unlock = useUnlockStore()
const sync = useSyncStore()
const ops = useOpsStore()
const shell = useShellStore()
const failed = computed(() => ops.finished.filter((o) => o.status === 'failed').length)
const isUnlocked = computed(() => !!repo.path && unlock.unlocked.has(repo.path))
// A command found it locked and it has not been unlocked since.
const isLocked = computed(() => !!repo.path && !isUnlocked.value && unlock.locked.has(repo.path))
const { openPath, pickAndOpen } = useOpenRepo()
const router = useRouter()

const repoName = computed(() => recent.list.find((r) => r.path === repo.path)?.name ?? repo.path ?? '')
const branch = computed(() => repo.status?.branch)
const up = computed(() => branch.value?.upstream)
const subline = computed(() => {
  const b = branch.value
  const where = b?.name ?? (b?.detached ? `detached ${b.detached_oid?.slice(0, 7)}` : 'no versions yet')
  const bits = [where]
  if (up.value?.ahead) bits.push(`${up.value.ahead} to send`)
  if (up.value?.behind) bits.push(`${up.value.behind} to get`)
  return bits.join(', ')
})

const menu = computed<MenuItem[]>(() => [
  ...recent.list.map((r) => ({ id: r.path, label: r.name })),
  { id: '__open', label: 'Open a repository…', separated: recent.list.length > 0 },
  { id: '__create', label: 'Create a new repository…' },
  { id: '__clone', label: 'Clone a repository…' },
])

function onMenu(id: string) {
  if (id === '__open') void pickAndOpen()
  else if (id === '__create') void router.push({ path: '/welcome', query: { action: 'create' } })
  else if (id === '__clone') void router.push({ path: '/welcome', query: { action: 'clone' } })
  else if (id !== repo.path) void openPath(id)
}

const settings = HUBS.find((h) => h.id === 'settings')!
// hubHome only names a hub's first section, so router-link-active misses the others.
const route = useRoute()
const onHub = (h: Hub) => inHub(route.path, h)
</script>

<template>
  <nav class="side" aria-label="Main">
    <div class="brand"><img :src="logo" alt="" width="22" height="22" /><span>MediaGit</span></div>

    <DropdownMenu :items="menu" @select="onMenu">
      <button type="button" class="switch" data-testid="repo-switcher">
        <span class="top"><span class="name">{{ repoName }}</span><ChevronsUpDown :size="16" aria-hidden="true" /></span>
        <span class="sub" data-testid="branch-indicator"><GitBranch :size="14" aria-hidden="true" />{{ subline }}</span>
      </button>
    </DropdownMenu>

    <p v-if="isUnlocked" class="enc" data-testid="unlocked-chip">
      <span>Encrypted, unlocked</span>
      <button type="button" class="link" aria-label="Lock this repository" @click="unlock.lock(repo.path!)">Lock</button>
    </p>
    <p v-else-if="isLocked" class="enc" data-testid="locked-chip">
      <span>Encrypted, locked</span>
      <button type="button" class="link" aria-label="Unlock this repository" @click="unlock.unlockRepo(repo.path!).catch(() => {})">Unlock…</button>
    </p>

    <ul class="items">
      <li v-for="(h, i) in WORK_HUBS" :key="h.id">
        <RouterLink :to="hubHome(h)" class="item" :class="{ on: onHub(h) }" :aria-current="onHub(h) ? 'page' : undefined" :aria-keyshortcuts="`Control+${i + 1}`">
          <component :is="h.icon" :size="20" aria-hidden="true" />
          <span class="lbl">{{ h.label }}</span>
          <span v-if="h.id === 'files' && changes.counts.total" class="count" :aria-label="`${changes.counts.total} changed ${changes.counts.total === 1 ? 'file' : 'files'}`">{{ changes.counts.total }}</span>
          <span v-else-if="h.id === 'team' && locks.locks.length" class="count" >{{ locks.locks.length }} {{ locks.locks.length === 1 ? 'lock' : 'locks' }}</span>
        </RouterLink>
      </li>
    </ul>

    <div class="foot">
      <RouterLink :to="hubHome(settings)" class="item" :class="{ on: onHub(settings) }" :aria-current="onHub(settings) ? 'page' : undefined" aria-keyshortcuts="Control+,">
        <component :is="settings.icon" :size="20" aria-hidden="true" />
        <span class="lbl">{{ settings.label }}</span>
      </RouterLink>
      <button
        type="button"
        class="act"
        aria-haspopup="dialog"
        :aria-expanded="shell.activityOpen"
        :aria-label="`Activity${failed ? `, ${failed} failed` : ''}${sync.last ? `. Got latest ${timeAgo(sync.last)}` : ''}`"
        @click="shell.activityOpen = !shell.activityOpen"
      >
        <template v-if="failed">{{ failed }} failed, </template>{{ sync.last ? `Got latest ${timeAgo(sync.last)}` : 'Activity' }}
      </button>
    </div>
  </nav>
</template>

<style scoped>
.side {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  width: var(--sidebar-w);
  flex: none;
  padding: var(--space-4) var(--space-3);
  overflow: hidden;
  border-right: 1px solid var(--border);
  background: var(--bg);
}
.brand {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 0 var(--space-2) var(--space-1);
  font-weight: 600;
  font-size: var(--fs-lg);
}
.switch {
  display: grid;
  gap: 2px;
  width: 100%;
  padding: var(--space-2) var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.switch:hover {
  background: var(--surface-2);
}
.top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-width: 0;
}
.name {
  min-width: 0;
  overflow: hidden;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}
.enc {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin: 0;
  padding: 0 var(--space-3);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.link {
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  font: inherit;
  font-weight: 600;
  text-decoration: underline;
  text-decoration-color: var(--brand);
  text-decoration-thickness: 2px;
  text-underline-offset: 3px;
  cursor: pointer;
}
.items {
  display: grid;
  gap: 2px;
  margin: var(--space-2) 0 0;
  padding: 0;
  list-style: none;
}
.item {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  height: var(--row-h-hub);
  padding: 0 var(--space-3);
  border-radius: var(--radius-control);
  color: var(--text);
  font: 400 var(--fs-md) / 1 var(--font-sans);
  text-decoration: none;
}
.item:hover {
  background: var(--surface-2);
}
.item.on {
  background: var(--accent-soft);
  font-weight: 600;
}
/* Selected hub: a brand bar inset at the left edge. */
.item.on::before {
  content: '';
  position: absolute;
  top: 8px;
  bottom: 8px;
  left: 0;
  width: 4px;
  border-radius: var(--radius-pill);
  background: var(--brand);
}
.lbl {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.count {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.foot {
  display: grid;
  gap: var(--space-2);
  margin-top: auto;
}
.act {
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  cursor: pointer;
}
.act:hover {
  background: var(--surface-2);
  color: var(--text);
}
.quiet {
  margin: 0;
  padding: 0 var(--space-3);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
