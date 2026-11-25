import { defineConfig } from '@playwright/test'

// Demo video recorder: drives the real release desktop app over CDP and writes one clip per
// chapter. Run with dev-tests/qa-suite/scripts/run_demo.ps1 (cleanup + stitch).
// No stale-build gate: qa-setup's check is not exported, so the runner trusts target/release.
export default defineConfig({
  testDir: 'tests/e2e/demo',
  testMatch: /.*\.demo\.ts/,
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 20 * 60_000,
  expect: { timeout: 15_000 },
  reporter: 'list',
  globalSetup: './tests/e2e/demo/demo-setup.ts',
  outputDir: 'test-results/demo/artifacts',
})
