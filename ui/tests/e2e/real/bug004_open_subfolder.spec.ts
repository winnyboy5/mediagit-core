import { test } from '@playwright/test'

// BUG-004, real app. PENDING: needs a release build at or after the BUG-004 commit
// (the coordinator builds releases). Un-fixme once target/release/mediagit.exe has it.
//
// Flow, on a repository with a subfolder `art/`:
//   1. Open the repository through the folder picker, choosing `art/`, not the root.
//   2. Assert the rail shows the repository (parent) name, not `art`.
//   3. Assert no `art/.mediagit` exists on disk afterwards.
test.describe('BUG-004: opening a subfolder opens the repository root', () => {
  test.fixme('the rail shows the parent name and no nested .mediagit appears', async () => {})
})
