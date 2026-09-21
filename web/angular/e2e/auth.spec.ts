import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { account, headers, login } from './auth-fixture';
import { readFileSync } from 'node:fs';

test('platform login, shared desktop/mobile data, revoked-session continuation and safe drafts', async ({
  page,
  context,
  browser,
}, info) => {
  const credentials = account();
  await page.goto('/drafts');
  await expect(page.getByRole('heading', { name: '登录 CodexSymphony' })).toBeVisible();
  expect(page.url()).toContain('/login?return=');
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: info.outputPath('login.png'), fullPage: true });
  await page.getByLabel('用户名', { exact: true }).fill(credentials.username);
  await page.getByLabel('密码', { exact: true }).fill(credentials.password);
  await page.getByRole('button', { name: '登录', exact: true }).click();
  await expect(page).toHaveURL(/\/drafts$/);
  const cookie = (await context.cookies()).find((cookie) => cookie.name === '__Host-codexsession')!;
  expect(cookie.httpOnly && cookie.secure && cookie.sameSite === 'Lax').toBe(true);
  expect(await page.evaluate(() => document.cookie)).not.toContain('__Host-codexsession');
  const other = await browser.newContext({
    baseURL: process.env['E2E_HTTPS_ORIGIN'],
    viewport: { width: 393, height: 851 },
  });
  try {
    await login(other, credentials);
    const draftDocument = JSON.parse(
      readFileSync('../../apps/server/tests/fixtures/draft-v1.json', 'utf8'),
    ) as { children: { repository_id: number | null }[] };
    for (const child of draftDocument.children) child.repository_id = null;
    const source = {
      format: 'json',
      text: JSON.stringify(draftDocument),
      label: 'shared-auth-fixture',
    };
    const created = await context.request.post('/api/drafts', {
      headers: await headers(context),
      data: { version: 0, source },
    });
    expect(created.status()).toBe(200);
    const draft = (await created.json()) as { id: string };
    const shared = await other.request.get(`/api/drafts/${draft.id}`);
    expect(shared.status()).toBe(200);
    expect((await shared.json()).id).toBe(draft.id);
    await page.getByRole('button', { name: '新建导入草稿', exact: true }).click();
    await page.getByLabel('草稿原文', { exact: true }).fill(source.text);
    const revoked = await context.request.post('/api/auth/logout', {
      headers: await headers(context),
      data: {},
    });
    expect(revoked.status()).toBe(204);
    let writes = 0;
    page.on('request', (request) => {
      if (request.method() === 'POST' && request.url().endsWith('/api/drafts')) writes++;
    });
    await page.getByRole('button', { name: '保存父子 Draft', exact: true }).click();
    await expect(page.getByRole('heading', { name: '登录 CodexSymphony' })).toBeVisible();
    await expect(page.getByRole('status')).toContainText('未提交输入保留');
    await page.getByLabel('用户名', { exact: true }).fill(credentials.username);
    await page.getByLabel('密码', { exact: true }).fill(credentials.password);
    await page.getByRole('button', { name: '登录', exact: true }).click();
    await expect(page.getByLabel('草稿原文', { exact: true })).toHaveValue(source.text);
    expect(writes).toBe(1);
    expect(await page.evaluate(() => localStorage.length + sessionStorage.length)).toBe(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({ path: info.outputPath('restored-draft.png'), fullPage: true });
  } finally {
    await other.close();
  }
});

test('rejects unsafe return targets and presents keyboard-accessible failed login', async ({
  page,
}) => {
  for (const target of [
    'https://evil.test',
    '//evil.test',
    '/%2f%2fevil.test',
    '/%255cevil.test',
  ]) {
    await page.goto(`/login?return=${encodeURIComponent(target)}`);
    const credentials = account();
    await page.getByLabel('用户名', { exact: true }).fill(credentials.username);
    await page.getByLabel('密码', { exact: true }).fill(credentials.password);
    await page.getByRole('button', { name: '登录', exact: true }).click();
    await expect(page).toHaveURL(`${process.env['E2E_HTTPS_ORIGIN']}/`);
    await page.getByRole('button', { name: '退出登录', exact: true }).click();
    await expect(page.getByRole('heading', { name: '登录 CodexSymphony' })).toBeVisible();
    await page.reload();
  }
  await page.emulateMedia({ reducedMotion: 'reduce', forcedColors: 'active' });
  await page.getByLabel('用户名', { exact: true }).fill('unknown');
  await page.getByLabel('密码', { exact: true }).fill('incorrect-password');
  await page.getByRole('button', { name: '登录', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('alert')).toContainText('用户名或密码错误');
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
