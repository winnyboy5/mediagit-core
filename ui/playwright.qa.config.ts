import { defineConfig } from '@playwright/test'

// Live QA campaign: drives the real desktop app over CDP with the release CLI and server on
// real media. Not part of CI. Run it with dev-tests/qa-suite/scripts/run_qalive.ps1 (verdict,
// gates.tsv, cleanup); see tests/e2e/qa-live/README.md. Raw: playwright test -c playwright.qa.config.ts -g "L-01"

export default defineConfig({
  testDir: 'tests/e2e/qa-live',
  testMatch: /.*\.spec\.ts/,
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 30 * 60_000,
  expect: { timeout: 15_000 },
  reporter: 'list',
  globalSetup: './tests/e2e/qa-live/qa-setup.ts',
  outputDir: 'test-results/qa-live/artifacts',
})
