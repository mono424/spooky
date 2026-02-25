import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI
    ? [['html', { open: 'never' }], ['github']]
    : [['html', { open: 'on-failure' }]],

  use: {
    baseURL: 'http://localhost:3006',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
    actionTimeout: 15_000,
  },

  webServer: [
    {
      command: 'bash scripts/start-db.sh',
      url: 'http://localhost:8666/health',
      reuseExistingServer: !process.env.CI,
      timeout: 180_000,
    },
    {
      command: 'pnpm dev',
      cwd: '../api',
      url: 'http://localhost:3660',
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
    {
      command: 'pnpm dev',
      cwd: '../app-solid',
      url: 'http://localhost:3006',
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
  ],

  projects: [
    {
      name: 'setup',
      testMatch: 'db-setup.spec.ts',
    },
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
      dependencies: ['setup'],
    },
  ],
});
