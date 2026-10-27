import { defineConfig, devices } from '@playwright/test'

// Mocked suite: the Vite dev server with Tauri IPC replaced by fixtures
// (tests/e2e/mocked/ipc-mock.ts). Runs on any OS, no Rust build needed.
// The real-app suite (tests/e2e/real) attaches to WebView2 over CDP and
// is run separately on Windows.
export default defineConfig({
  testDir: 'tests/e2e/mocked',
  fullyParallel: true,
  // Each worker is its own Chromium process. On the Windows dev host (AV scanning each
  // new browser process) 3+ browsers starting together stall page loads for 30 s+, so
  // the default is capped; CI can raise it with --workers.
  workers: 2,
  forbidOnly: !!process.env.CI,
  retries: 1,
  reporter: process.env.CI ? 'github' : 'list',
  use: { baseURL: 'http://localhost:5173', trace: 'retain-on-failure' },
  projects: [
    { name: 'light', use: { ...devices['Desktop Chrome'], colorScheme: 'light', viewport: { width: 1440, height: 900 } } },
    { name: 'dark', use: { ...devices['Desktop Chrome'], colorScheme: 'dark', viewport: { width: 1440, height: 900 } } },
  ],
  webServer: { command: 'pnpm dev', url: 'http://localhost:5173', reuseExistingServer: !process.env.CI },
})
