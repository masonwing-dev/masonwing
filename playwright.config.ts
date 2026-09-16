import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/web/e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30000,
  reporter: [['list'], ['json', { outputFile: '.dev/evidence/playwright-results.json' }]],
  outputDir: '.dev/evidence/browser',
  use: { baseURL: 'http://localhost:39850', browserName: 'chromium', trace: 'retain-on-failure', screenshot: 'only-on-failure' },
});
