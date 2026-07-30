import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  timeout: 40000,
  use: {
    channel: 'chrome',
    headless: true,
  },
});
