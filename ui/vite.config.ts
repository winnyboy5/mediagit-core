/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

// Tauri loads devUrl http://localhost:5173 (crates/mediagit-desktop/tauri.conf.json),
// so the port is fixed and must fail loudly if taken.
export default defineConfig({
  plugins: [vue()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  clearScreen: false,
  // warmup: transform the app graph at startup, so the first page loads
  // (parallel Playwright workers) don't race Vite's on-demand compile.
  server: { port: 5173, strictPort: true, warmup: { clientFiles: ['./src/main.ts', './src/**/*.{vue,ts}'] } },
  // Pre-bundle every dependency up front: a dep discovered mid-run makes Vite
  // re-optimize and reload the page, which hangs parallel Playwright workers.
  optimizeDeps: {
    include: [
      'reka-ui',
      'lucide-vue-next',
      'pinia',
      'vue-router',
      '@tanstack/vue-virtual',
      '@tauri-apps/api/core',
      '@tauri-apps/api/event',
      '@tauri-apps/api/window',
      '@tauri-apps/plugin-dialog',
    ],
  },
  build: { target: 'es2022', sourcemap: false },
  test: {
    environment: 'happy-dom',
    include: ['tests/unit/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      include: ['src/stores/**', 'src/lib/**', 'src/api/**'],
      exclude: ['src/api/generated/**', 'src/api/types.ts'],
      reporter: ['text-summary', 'json-summary'],
      reportOnFailure: true,
      // CI gate: `vitest run --coverage` fails when any of these dirs drops below 85% of lines.
      thresholds: {
        'src/stores/**': { lines: 85 },
        'src/lib/**': { lines: 85 },
        'src/api/**': { lines: 85 },
      },
    },
  },
})
