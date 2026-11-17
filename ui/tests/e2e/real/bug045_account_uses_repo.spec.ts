import { test } from '@playwright/test'

// BUG-045, real app. PENDING: needs a release build at or after the BUG-045 commit
// (the coordinator builds releases). Un-fixme once target/release/mediagit.exe has it.
//
// Flow, on a repository whose remote is a running local server with sign-up open:
//   1. Security, "Create account...": leave the server address empty, fill user, email, password.
//      The account is created on the repository's remote (before the fix: "no server given").
//   2. Sign in with it, then "Change password...": leave the address empty, change the password.
//   3. Assert with the CLI: `mediagit auth login` with the new password succeeds, the old one is refused.
test.describe('BUG-045: create account and change password use the repository remote', () => {
  test.fixme('both dialogs work with the address field empty', async () => {})
})
