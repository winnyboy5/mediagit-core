import { defineAsyncComponent, type Component } from 'vue'
import { Folder, GitBranch, History as HistoryIcon, Settings, Users } from 'lucide-vue-next'
import { useChangesStore } from '@/stores/changes'
import { useLocksStore } from '@/stores/locks'
import { useMergeStore } from '@/stores/merge'
import { useTagsStashStore } from '@/stores/tagsStash'

export type HubId = 'files' | 'history' | 'branches' | 'team' | 'settings'

export interface HubSection {
  id: string
  label: string
  component: Component
  /** Props handed to the section component. */
  props?: Record<string, unknown>
  /** Redirect to Welcome when no repository is open (Server works without one). */
  needsRepo: boolean
  /** Shows the file inspector beside the section. */
  inspector?: boolean
  /** A number shown beside the label ("Changes 9"); nothing when 0 or unknown. */
  count?: () => number | undefined
}

export interface Hub {
  id: HubId
  label: string
  icon: Component
  /** `segmented` = one control in the header; `list` = left list (Settings). */
  layout: 'segmented' | 'list'
  /** Get latest / Send in the header. */
  sync: boolean
  sections: HubSection[]
}

const lazy = (load: () => Promise<{ default: Component }>) => defineAsyncComponent(load)
const Files = lazy(() => import('@/views/FilesView.vue'))
const Changes = lazy(() => import('@/views/ChangesView.vue'))
const History = lazy(() => import('@/views/HistoryView.vue'))
const Compare = lazy(() => import('@/views/CompareView.vue'))
const TagsStash = lazy(() => import('@/views/TagsStashView.vue'))
const Branches = lazy(() => import('@/views/BranchesView.vue'))
const Merge = lazy(() => import('@/views/MergeView.vue'))
const Sync = lazy(() => import('@/views/SyncView.vue'))
const Locks = lazy(() => import('@/views/LocksView.vue'))
const Remote = lazy(() => import('@/views/RemoteView.vue'))
const Server = lazy(() => import('@/views/ServerView.vue'))
const Settings_ = lazy(() => import('@/views/SettingsView.vue'))
const Storage = lazy(() => import('@/views/StorageView.vue'))
const Maintenance = lazy(() => import('@/views/MaintenanceView.vue'))
const Security = lazy(() => import('@/views/SecurityView.vue'))
const Advanced = lazy(() => import('@/views/AdvancedView.vue'))

export const HUBS: Hub[] = [
  {
    id: 'files',
    label: 'Files',
    icon: Folder,
    layout: 'segmented',
    sync: true,
    sections: [
      { id: 'all', label: 'All files', component: Files, needsRepo: true, inspector: true },
      { id: 'changes', label: 'Changes', component: Changes, needsRepo: true, inspector: true, count: () => useChangesStore().counts.total },
    ],
  },
  {
    id: 'history',
    label: 'History',
    icon: HistoryIcon,
    layout: 'segmented',
    sync: true,
    sections: [
      { id: 'versions', label: 'Versions', component: History, needsRepo: true, inspector: true },
      { id: 'compare', label: 'Compare', component: Compare, needsRepo: true },
      { id: 'tags', label: 'Tags', component: TagsStash, props: { only: 'tags' }, needsRepo: true, count: () => useTagsStashStore().tags.length },
    ],
  },
  {
    id: 'branches',
    label: 'Branches',
    icon: GitBranch,
    layout: 'segmented',
    sync: true,
    sections: [
      { id: 'branches', label: 'Branches', component: Branches, needsRepo: true },
      { id: 'merge', label: 'Merge', component: Merge, needsRepo: true, count: () => useMergeStore().conflicts.length },
      { id: 'sync', label: 'Sync', component: Sync, needsRepo: true },
      { id: 'stash', label: 'Stash', component: TagsStash, props: { only: 'stash' }, needsRepo: true, count: () => useTagsStashStore().stashes.length },
    ],
  },
  {
    id: 'team',
    label: 'Team',
    icon: Users,
    layout: 'segmented',
    sync: true,
    sections: [
      { id: 'locks', label: 'Locks', component: Locks, needsRepo: true, count: () => useLocksStore().locks.length },
      { id: 'remote', label: 'Remote files', component: Remote, needsRepo: true },
      { id: 'server', label: 'Server', component: Server, needsRepo: false },
    ],
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: Settings,
    layout: 'list',
    sync: false,
    sections: [
      { id: 'general', label: 'General', component: Settings_, needsRepo: true },
      { id: 'storage', label: 'Storage', component: Storage, needsRepo: true },
      { id: 'maintenance', label: 'Maintenance', component: Maintenance, needsRepo: true },
      { id: 'security', label: 'Security', component: Security, needsRepo: true },
      { id: 'advanced', label: 'Advanced tools', component: Advanced, needsRepo: true },
    ],
  },
]

/** A hub's landing path: its first section, spelled out so the URL says where you are. */
export const hubHome = (h: Hub) => `/${h.id}/${h.sections[0]!.id}`

/** Whether a route path is inside a hub, in any of its sections (hubHome only names the first). */
export const inHub = (path: string, h: Hub) => path.startsWith(`/${h.id}/`)

export const WORK_HUBS = HUBS.filter((h) => h.id !== 'settings')

export const hubById = (id: string | undefined) => HUBS.find((h) => h.id === id)

/** The section a path lands on; the hub's first when the id is missing or unknown. */
export function sectionOf(hub: Hub, id: string | undefined): HubSection {
  return hub.sections.find((s) => s.id === id) ?? hub.sections[0]!
}

/** Where an old top-level route now lives (deep links, bookmarks), plus the bare hub paths. */
export const LEGACY_ROUTES: Record<string, string> = {
  '/files': '/files/all',
  '/team': '/team/locks',
  '/changes': '/files/changes',
  '/history': '/history/versions',
  '/compare': '/history/compare',
  '/tags': '/history/tags',
  '/branches': '/branches/branches',
  '/merge': '/branches/merge',
  '/sync': '/branches/sync',
  '/locks': '/team/locks',
  '/remote': '/team/remote',
  '/server': '/team/server',
  '/storage': '/settings/storage',
  '/maintenance': '/settings/maintenance',
  '/security': '/settings/security',
  '/advanced': '/settings/advanced',
  '/settings': '/settings/general',
}
