import { createRouter, createWebHashHistory } from 'vue-router'
import { HUBS, LEGACY_ROUTES, hubById, sectionOf } from './shell/hubs'
import { useRepoStore } from './stores/repo'

declare module 'vue-router' {
  interface RouteMeta {
    /** `none` hides the sidebar (Welcome). */
    chrome?: 'none'
    /** Standalone page outside the app shell. */
    bare?: boolean
    /** Hub pages: which hub this route shows. */
    hub?: string
    title?: string
  }
}

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: () => (useRepoStore().path ? '/files/all' : '/welcome') },
    { path: '/welcome', component: () => import('./views/WelcomeView.vue'), meta: { chrome: 'none', title: 'Welcome' } },
    // Old top-level routes keep working as deep links. First, so /history etc. redirect
    // instead of matching the hub's optional section.
    ...Object.entries(LEGACY_ROUTES).map(([from, to]) => ({
      path: from,
      redirect: (r: { query: Record<string, unknown> }) => ({ path: to, query: r.query as Record<string, string> }),
    })),
    // Five hubs; each hub page picks its section from the path.
    ...HUBS.map((h) => ({
      // Named sections only, so a bare old path like /history is not swallowed by its hub.
      path: `/${h.id}/:section(${h.sections.map((s) => s.id).join('|')})/:sub?`,
      component: () => import('./shell/HubView.vue'),
      meta: { hub: h.id, title: h.label },
    })),
    // Tags and Stash on one page; the hubs show them apart (History > Tags, Branches > Stash).
    { path: '/tags-stash', component: () => import('./views/TagsStashView.vue'), beforeEnter: () => (useRepoStore().path ? true : '/welcome'), meta: { title: 'Tags & Stash' } },
    // Component gallery: dev builds only, so the shipped app has no /kit.
    ...(import.meta.env.DEV ? [{ path: '/kit', component: () => import('./kit/KitView.vue'), meta: { bare: true } }] : []),
  ],
})

router.beforeEach((to) => {
  const hub = hubById(to.meta.hub)
  // Most sections need an open repository; Server works without one.
  if (hub && sectionOf(hub, to.params.section as string | undefined).needsRepo && !useRepoStore().path) return '/welcome'
})
