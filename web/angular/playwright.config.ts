import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: process.env['E2E_HTTPS_ORIGIN'] ?? 'http://127.0.0.1:4300',
    launchOptions: {
      args: process.env['E2E_TLS_SPKI']
        ? [`--ignore-certificate-errors-spki-list=${process.env['E2E_TLS_SPKI']}`]
        : [],
    },
  },
  projects: [
    { name: 'desktop', use: { ...devices['Desktop Chrome'] } },
    { name: 'mobile', use: { ...devices['Pixel 7'] } },
  ],
  webServer: process.env['E2E_HTTPS_ORIGIN']
    ? undefined
    : {
        command: 'npm start -- --host 127.0.0.1 --port 4300 --proxy-config proxy.e2e.conf.cjs',
        url: 'http://127.0.0.1:4300',
        reuseExistingServer: false,
        timeout: 120000,
      },
});
