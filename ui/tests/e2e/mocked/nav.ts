import { expect, type Page } from '@playwright/test'

// One way for every screenshot / review spec to reach a section: by its hub path.
// Old top-level routes (/changes, /server ...) map to the hub that now holds them.
const HUB_LABEL: Record<string, string> = { files: 'Files', history: 'History', branches: 'Branches', team: 'Team', settings: 'Settings' }
const MOVED: Record<string, string> = {
  changes: 'files/changes',
  'tags-stash': 'branches/stash',
  history: 'history/versions',
  compare: 'history/compare',
  tags: 'history/tags',
  branches: 'branches/branches',
  merge: 'branches/merge',
  sync: 'branches/sync',
  locks: 'team/locks',
  remote: 'team/remote',
  server: 'team/server',
  storage: 'settings/storage',
  maintenance: 'settings/maintenance',
  security: 'settings/security',
  advanced: 'settings/advanced',
  settings: 'settings/general',
}

/** `hub/section` for an old or new route, with or without a leading slash. */
export function sectionPath(route: string): string {
  const r = route.replace(/^\/?#?\/?/, '')
  return MOVED[r] ?? r
}

/** Go to a section, wait for its hub title (the only h1) and, when given, the section's own h2. */
export async function openSection(page: Page, route: string, heading?: string, opts: { waitUntil?: 'domcontentloaded' } = { waitUntil: 'domcontentloaded' }) {
  const path = sectionPath(route)
  await page.goto(`/#/${path}`, opts)
  await expect(page.getByRole('heading', { name: HUB_LABEL[path.split('/')[0]!] ?? /.+/, level: 1 })).toBeVisible()
  // Tags & Stash and Settings were renamed by the hubs; their old titles no longer exist.
  if (heading && !['Tags & Stash', 'Settings'].includes(heading)) await expect(page.getByRole('heading', { name: heading, level: 2, exact: true }).first()).toBeVisible()
}
