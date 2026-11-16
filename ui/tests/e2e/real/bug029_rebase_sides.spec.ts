import { test } from '@playwright/test'

// BUG-029, real app. PENDING: needs a release build at or after the BUG-029 commit
// (the coordinator builds releases). Un-fixme once target/release/mediagit.exe has it.
//
// Flow, run twice on fresh repos (once per side), checking the CLI after each:
//   1. main edits notes.txt; feature edits the same lines; the app starts "Rebase onto main"
//      from feature. It stops on notes.txt.
//   2. Changes shows "A rebase is in progress." with a "Decide the files" link to Advanced tools.
//   3. Advanced tools, Rebase: the buttons read "Keep main" (the branch built on) and
//      "Keep feature" (the branch being moved), never "Keep mine".
//   4. Click one, then "Continue rebase". The rebase finishes.
//   5. Assert with `cat notes.txt`: "Keep main" gives main's text, "Keep feature" gives feature's.
//      The two runs must differ, and neither may fail because the bytes equal HEAD's
//      (before the fix, "Keep main" left the file unresolved and Continue was refused).
test.describe('BUG-029: finish a rebase conflict with each side', () => {
  test.fixme('keep main, then keep feature: results differ and both finish', async () => {})
})
