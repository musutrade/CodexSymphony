import { test as base, expect, type BrowserContext } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, writeFileSync, openSync, closeSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export function account(command = 'init', username = `browser-${randomBytes(12).toString('hex')}`) {
  const password = randomBytes(32).toString('hex');
  const directory = mkdtempSync(join(tmpdir(), 'auth-input-'));
  const path = join(directory, 'input.json');
  writeFileSync(path, JSON.stringify({ username, password }), { mode: 0o600 });
  const fd = openSync(path, 'r');
  try {
    const result = spawnSync(process.env['E2E_AUTH_BINARY']!, ['auth', command, '--stdin-json'], {
      stdio: [fd, 'pipe', 'pipe'],
    });
    if (result.status !== 0) throw new Error('Disposable test account bootstrap failed');
  } finally {
    closeSync(fd);
    rmSync(directory, { recursive: true });
  }
  return { username, password };
}
export async function headers(context: BrowserContext) {
  const response = await context.request.get('/api/auth/csrf');
  expect(response.status()).toBe(200);
  const proof = (await response.json()) as { csrf_token: string };
  return { origin: process.env['E2E_HTTPS_ORIGIN']!, 'x-codexsymphony-csrf': proof.csrf_token };
}
export async function login(
  context: BrowserContext,
  credentials: { username: string; password: string },
) {
  const response = await context.request.post('/api/auth/login', {
    headers: await headers(context),
    data: credentials,
  });
  expect(response.status()).toBe(200);
}
export const test = base.extend<{ authenticated: void }>({
  authenticated: [
    async ({ context }, use) => {
      await login(context, account());
      await use();
    },
    { auto: true },
  ],
});
export { expect };
