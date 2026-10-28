<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'
import {
  ArrowDown,
  ArrowUp,
  ChevronsUpDown,
  GitBranch,
  GitCompare,
  GitMerge,
  HardDrive,
  History as HistoryIcon,
  Lock,
  RefreshCw,
  Settings,
  ShieldCheck,
  Tag,
  Wrench,
  Cloud,
  FileDiff,
  Wand2,
} from 'lucide-vue-next'
import logo from '@/assets/logo-16.svg'
import DropdownMenu, { type MenuItem } from '@/components/DropdownMenu.vue'
import { useChangesStore } from '@/stores/changes'
import { useRecentStore } from '@/stores/recent'
import { useRepoStore } from '@/stores/repo'
import { useUnlockStore } from '@/stores/unlock'
import { useOpenRepo } from './useOpenRepo'

const repo = useRepoStore()
const recent = useRecentStore()
const changes = useChangesStore()
const unlock = useUnlockStore()
const isUnlocked = computed(() => !!repo.path && unlock.unlocked.has(repo.path))
const { openPath, pickAndOpen } = useOpenRepo()

const repoName = computed(() => recent.list.find((r) => r.path === repo.path)?.name ?? repo.path ?? '')
const branch = computed(() => repo.status?.branch)
const up = computed(() => branch.value?.upstream)

const menu = computed<MenuItem[]>(() => [
  ...recent.list.map((r) => ({ id: r.path, label: r.name })),
  { id: '__open', label: 'Open another repository…', separated: recent.list.length > 0 },
])

function onMenu(id: string) {
  if (id === '__open') void pickAndOpen()
  else if (id !== repo.path) void openPath(id)
}

const nav = [
  { to: '/changes', label: 'Changes', icon: FileDiff },
  { to: '/history', label: 'History', icon: HistoryIcon },
  { to: '/branches', label: 'Branches', icon: GitBranch },
  { to: '/locks', label: 'Locks', icon: Lock },
  { to: '/sync', label: 'Sync', icon: RefreshCw },
  { to: '/merge', label: 'Merge & conflicts', icon: GitMerge },
  { to: '/compare', label: 'Compare', icon: GitCompare },
  { to: '/tags', label: 'Tags & Stash', icon: Tag },
  { to: '/remote', label: 'Remote Browser', icon: Cloud },
  { to: '/storage', label: 'Storage', icon: HardDrive },
  { to: '/maintenance', label: 'Maintenance', icon: Wrench },
  { to: '/security', label: 'Security', icon: ShieldCheck },
  { to: '/advanced', label: 'Advanced', icon: Wand2 },
  { to: '/settings', label: 'Settings', icon: Settings },
] as const

</script>

<template>
  <nav class="side" aria-label="Main">
    <div class="brand"><img :src="logo" alt="" width="22" height="22" /><span>MediaGit</span></div>

    <DropdownMenu :items="menu" @select="onMenu">
      <button type="button" class="switch" data-testid="repo-switcher">
        <span class="cap">Repository</span>
        <span class="val"><span class="name">{{ repoName }}</span><ChevronsUpDown :size="16" aria-hidden="true" /></span>
      </button>
    </DropdownMenu>

    <div class="switch static" data-testid="branch-indicator">
      <span class="cap">Branch</span>
      <span class="val branchline">
        <GitBranch :size="16" aria-hidden="true" />
        <span v-if="branch?.name" class="name mono">{{ branch.name }}</span>
        <span v-else-if="branch?.detached" class="name mono">detached {{ branch.detached_oid?.slice(0, 7) }}</span>
        <span v-else class="name muted">no commits yet</span>
        <span v-if="up && (up.ahead || up.behind)" class="sync mono" :aria-label="`${up.ahead ?? 0} ahead, ${up.behind ?? 0} behind`">
          <template v-if="up.ahead"><ArrowUp :size="14" aria-hidden="true" />{{ up.ahead }}</template><template v-if="up.behind"><ArrowDown :size="14" aria-hidden="true" />{{ up.behind }}</template>
        </span>
      </span>
    </div>

    <div v-if="isUnlocked" class="switch static" data-testid="unlocked-chip">
      <span class="cap">Encryption</span>
      <span class="val">
        <span class="name">Unlocked</span>
        <button type="button" class="lockbtn" aria-label="Lock this repository" @click="unlock.lock(repo.path!)">Lock</button>
      </span>
    </div>

    <div class="navscroll">
    <ul class="items">
      <li v-for="(n, i) in nav" :key="n.to">
        <RouterLink :to="n.to" class="item" :aria-keyshortcuts="i < 9 ? `Control+${i + 1}` : undefined">
          <component :is="n.icon" :size="18" aria-hidden="true" />
          <span class="lbl">{{ n.label }}</span>
          <span v-if="n.to === '/changes' && changes.counts.total" class="count mono" :aria-label="`${changes.counts.total} changed files`">{{
            changes.counts.total
          }}</span>
          <span v-else-if="n.to === '/sync' && up && (up.ahead || up.behind)" class="count mono" aria-hidden="true">
            <template v-if="up.ahead"><ArrowUp :size="14" aria-hidden="true" />{{ up.ahead }}</template><template v-if="up.behind"><ArrowDown :size="14" aria-hidden="true" />{{ up.behind }}</template>
          </span>
        </RouterLink>
      </li>
    </ul>

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
  border: 1px solid var(--border);
  border-radius: var(--radius-panel);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  text-align: left;
}
button.switch {
  cursor: pointer;
}
button.switch:hover {
  background: var(--surface-2);
}
.cap {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.val {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  min-width: 0;
}
.branchline {
  justify-content: flex-start;
}
.name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}
.lockbtn {
  padding: 0 var(--space-2);
  border: 0;
  background: transparent;
  color: var(--accent);
  font: inherit;
  font-weight: 600;
  text-decoration: underline;
  cursor: pointer;
}
.muted {
  color: var(--text-muted);
  font-weight: 400;
}
.sync,
.count:has(svg) {
  display: inline-flex;
  align-items: center;
  gap: 2px;
}
.sync {
  margin-left: auto;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.navscroll {
  display: grid;
  flex: 1;
  align-content: start;
  gap: var(--space-3);
  min-height: 0;
  padding: var(--space-2) 0 var(--space-4);
  overflow-y: auto;
  mask-image: linear-gradient(to bottom, transparent, #000 var(--space-2), #000 calc(100% - var(--space-4)), transparent);
}
.items {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}
.item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  width: 100%;
  height: var(--control-h);
  padding: 0 var(--space-3);
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font: 400 var(--fs-md) / 1 var(--font-sans);
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}
.item:hover {
  background: var(--surface-2);
}
.item.router-link-active {
  background: var(--accent-soft);
  font-weight: 600;
}
.item.soon {
  color: var(--text-muted);
  cursor: default;
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
.group {
  margin: var(--space-3) 0 0;
  padding: 0 var(--space-3);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}
</style>
