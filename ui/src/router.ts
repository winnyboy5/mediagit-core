import { createRouter, createWebHashHistory } from 'vue-router'
import { useRepoStore } from './stores/repo'

declare module 'vue-router' {
  interface RouteMeta {
    /** `none` hides the sidebar (Welcome). */
    chrome?: 'none'
    /** Standalone page outside the app shell. */
    bare?: boolean
    /** File views show the inspector panel. */
    inspector?: boolean
    /** Redirect to Welcome when no repository is open. */
    needsRepo?: boolean
    title?: string
  }
}

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: () => (useRepoStore().path ? '/changes' : '/welcome') },
    { path: '/welcome', component: () => import('./views/WelcomeView.vue'), meta: { chrome: 'none', title: 'Welcome' } },
    { path: '/changes', component: () => import('./views/ChangesView.vue'), meta: { needsRepo: true, inspector: true, title: 'Changes' } },
    { path: '/history', component: () => import('./views/HistoryView.vue'), meta: { needsRepo: true, inspector: true, title: 'History' } },
    { path: '/branches', component: () => import('./views/BranchesView.vue'), meta: { needsRepo: true, title: 'Branches' } },
    { path: '/locks', component: () => import('./views/LocksView.vue'), meta: { needsRepo: true, title: 'Locks' } },
    { path: '/sync', component: () => import('./views/SyncView.vue'), meta: { needsRepo: true, title: 'Sync' } },
    { path: '/merge', component: () => import('./views/MergeView.vue'), meta: { needsRepo: true, title: 'Merge & conflicts' } },
    { path: '/compare', component: () => import('./views/CompareView.vue'), meta: { needsRepo: true, title: 'Compare' } },
    { path: '/tags', component: () => import('./views/TagsStashView.vue'), meta: { needsRepo: true, title: 'Tags & Stash' } },
    { path: '/remote', component: () => import('./views/RemoteView.vue'), meta: { needsRepo: true, title: 'Remote Browser' } },
    { path: '/storage', component: () => import('./views/StorageView.vue'), meta: { needsRepo: true, title: 'Storage' } },
    { path: '/maintenance', component: () => import('./views/MaintenanceView.vue'), meta: { needsRepo: true, title: 'Maintenance' } },
    { path: '/security', component: () => import('./views/SecurityView.vue'), meta: { needsRepo: true, title: 'Security' } },
    { path: '/server', component: () => import('./views/ServerView.vue'), meta: { title: 'Server' } },
    { path: '/advanced', component: () => import('./views/AdvancedView.vue'), meta: { needsRepo: true, title: 'Advanced' } },
    { path: '/settings', component: () => import('./views/SettingsView.vue'), meta: { needsRepo: true, title: 'Settings' } },
    { path: '/kit', component: () => import('./kit/KitView.vue'), meta: { bare: true } },
  ],
})

router.beforeEach((to) => {
  if (to.meta.needsRepo && !useRepoStore().path) return '/welcome'
})
