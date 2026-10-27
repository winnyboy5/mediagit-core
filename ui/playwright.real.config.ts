import { defineConfig } from '@playwright/test'

// Real-app suite (gate G4): Playwright attaches over CDP to the actual Tauri app
// (WebView2) running against a real fixture repo, a real local mediagit-server and
// the real CLI. tests/e2e/real/setup.ts owns every process, so there is no webServer.
// Windows only. Run: pnpm e2e:real  (debug builds of cli, server and desktop first).
export default defineConfig({
  testDir: 'tests/e2e/real',
  testMatch: /.*\.spec\.ts/,
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  forbidOnly: !!process.env.CI,
  reporter: 'list',
  globalSetup: './tests/e2e/real/setup.ts',
  outputDir: 'test-results/real/artifacts',
})
