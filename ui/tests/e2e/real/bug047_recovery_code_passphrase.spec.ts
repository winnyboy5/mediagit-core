import { test } from '@playwright/test'

// BUG-047, real app. PENDING: needs a release build at or after the BUG-047 commit
// (the coordinator builds releases). Un-fixme once target/release/mediagit.exe has it.
//
// Flow, on a new empty repository:
//   1. Security > "Turn on encryption..." > type a passphrase > "Turn on encryption".
//   2. Sample the screen every 150 ms for 12 s without touching it: the "Save your recovery code"
//      dialog is on screen from the first sample (before the fix: never, only the unlock prompt).
//   3. Tick "I saved it", Done, then answer the unlock prompt with the passphrase: it unlocks.
//   Also check the order the other way: unlock first, then the code dialog is still there.
//   (T-08b saw no code even after unlocking; a Vitest through the real invoke wrapper and unlock
//   store showed the code after unlock, so only this real-app run can settle that part.)
test.describe('BUG-047: the recovery code shows with a passphrase', () => {
  test.fixme('code dialog appears before and survives the unlock prompt', async () => {})
})
