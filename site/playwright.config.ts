import {defineConfig, devices} from '@playwright/test';
import {deployment} from './scripts/deployment.mjs';

const {base} = deployment();

export default defineConfig({
  testDir: './tests',
  workers: 1,
  reporter: 'list',
  use: {
    baseURL: 'http://127.0.0.1:4321',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'node scripts/serve.mjs 4321',
    url: `http://127.0.0.1:4321${base}`,
    reuseExistingServer: !process.env.CI,
    timeout: 120000,
  },
  projects: [
    {name: 'desktop', use: {...devices['Desktop Chrome'], viewport: {width: 1280, height: 800}}},
    {name: 'mobile', use: {...devices['Pixel 7'], viewport: {width: 390, height: 844}}},
  ],
});
