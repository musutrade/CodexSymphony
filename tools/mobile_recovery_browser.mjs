// Actual Chromium actions against the owned HTTPS service; no route mocks.
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
const require = createRequire(new URL('../web/angular/package.json', import.meta.url));
const { chromium, expect } = require('@playwright/test');
const { default: AxeBuilder } = require('@axe-core/playwright');
const input = JSON.parse(readFileSync(0, 'utf8'));
const browser = await chromium.launch({ args: [`--ignore-certificate-errors-spki-list=${input.spki}`] });
try {
  const context = await browser.newContext({ baseURL: input.origin, viewport: input.step === 'create' ? { width: 1280, height: 900 } : { width: 390, height: 844 }, isMobile: input.step !== 'create', hasTouch: input.step !== 'create' });
  const csrf = await context.request.get('/api/auth/csrf');
  let headers = { origin: input.origin, 'x-codexsymphony-csrf': (await csrf.json()).csrf_token };
  const login = await context.request.post('/api/auth/login', { headers, data: input.account });
  expect(login.status()).toBe(200);
  headers = { origin: input.origin, 'x-codexsymphony-csrf': (await login.json()).csrf_token };
  const page = await context.newPage();
  let id = input.id;
  if (input.step === 'create') {
    const scenarios = JSON.parse(readFileSync(new URL('../api/capture-scenarios.json', import.meta.url)));
    const configured = scenarios.find(v => v.id === 'configured').body;
    configured.repository.model = 'gpt-6-astra';
    const repo = await context.request.put('/api/repository', { headers, data: configured });
    expect(repo.status()).toBe(200);
    const created = await context.request.post('/api/requirements', { headers, data: scenarios.find(v => v.id === 'created').body });
    expect(created.status()).toBe(201);
    id = (await created.json()).id;
    const ready = await context.request.post(`/api/requirements/${id}/ready`, { headers, data: { request_id: 'mobile-reviewed-ready', version: 1, repository_version: 1 } });
    expect(ready.status()).toBe(200);
  }
  await page.goto(`/requirements/${id}`);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('需求详情与 Run 时间线');
  if (input.step === 'answer') {
    await page.getByLabel('隔夜继续使用原范围？', { exact: true }).fill('继续原范围');
    await page.getByRole('button', { name: '保存回答', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('操作已保存');
    const stale = await context.request.post(`/api/operator/questions/${input.question}/answer`, { headers, data: { version: 1, answers: [{ id: 'choice', text: '重复旧 RPC 回答' }] } });
    expect(stale.status()).toBe(409);
  } else if (input.step === 'pause' || input.step === 'resume') {
    await page.getByRole('button', { name: input.step === 'pause' ? '暂停' : '恢复', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('操作已保存');
  } else if (input.step === 'cancel') {
    await page.getByRole('button', { name: '取消需求', exact: true }).click();
    await page.getByRole('button', { name: '确认取消并关闭关联 PR' }).click();
    await expect(page.getByRole('status')).toContainText('操作已保存');
  }
  expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: `${input.output}/${input.step}.png`, fullPage: true });
  const detail = await context.request.get(`/api/requirements/${id}/operations`);
  const saved = await detail.json();
  writeFileSync(`${input.output}/${input.step}.json`, JSON.stringify(saved, null, 2));
  console.log(JSON.stringify({ id }));
} finally { await browser.close(); }
