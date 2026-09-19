import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

// Ordinary project fixtures in the disposable database, never a production endpoint.
function fixture(id: number, suffix: string) {
  const url = process.env['TEST_DATABASE_URL'];
  if (!url) throw new Error('TEST_DATABASE_URL is required for persisted operator fixtures');
  const sql = readFileSync('../../api/capture-fixture.sql', 'utf8')
    .replaceAll('900001', String(id))
    .replaceAll('capture-', `browser-${suffix}-`);
  execFileSync('psql', [url, '-X', '-v', 'ON_ERROR_STOP=1', '--single-transaction'], {
    input: sql,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}

test('persisted inbox, six questions, paused answer, stale answer and cancellation survive reload', async ({
  page,
}, info) => {
  const suffix = `${info.project.name}-${Date.now()}`;
  const id = Date.now() + (info.project.name === 'desktop' ? 1 : 2);
  fixture(id, suffix);
  const question = `browser-${suffix}-question`;
  await page.goto(`/requirements/${id}`);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('需求详情与 Run 时间线');
  const card = page.getByRole('region', { name: '聚合阻塞卡' });
  for (const label of ['阶段', '原因确认程度', '已保存内容', '已尝试动作', '下一步', '恢复位置']) {
    await expect(card.locator('dt', { hasText: label })).toBeVisible();
  }
  await expect(page.getByText('暂停意图：已暂停', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: '保存回答', exact: true }).click();
  await expect(page.getByLabel('Which option?', { exact: true })).toHaveAttribute(
    'aria-describedby',
    /mat-mdc-error/,
  );
  await expect(page.locator(`#answer-error-${question}`)).toBeVisible();
  await page.getByLabel('Which option?', { exact: true }).fill('yes');
  const saved = page.waitForResponse((response) =>
    response.url().endsWith(`/api/operator/questions/${question}/answer`),
  );
  await page.getByRole('button', { name: '保存回答', exact: true }).click();
  const savedResponse = await saved;
  expect(savedResponse.status(), await savedResponse.text()).toBe(200);
  await expect(page.getByRole('status')).toContainText('操作已保存');
  await expect(page.getByRole('button', { name: '保存回答', exact: true })).toHaveCount(0);
  await expect(page.getByText('暂停意图：已暂停', { exact: false })).toBeVisible();
  const stale = await page.request.post(`/api/operator/questions/${question}/answer`, {
    headers: { origin: 'http://127.0.0.1:4300', 'x-codexsymphony-csrf': '1' },
    data: { version: 1, answers: [{ id: 'choice', text: 'different' }] },
  });
  expect(stale.status()).toBe(409);
  await page.getByRole('button', { name: '查看脱敏日志预览' }).click();
  await expect(page.getByRole('region', { name: '脱敏日志预览' })).toContainText(
    'Fixture-owned output',
  );
  await page.reload();
  await expect(page.getByRole('button', { name: '保存回答', exact: true })).toHaveCount(0);
  await page.goto('/inbox');
  await expect(page.getByRole('link', { name: `需求 #${id}`, exact: true })).toBeVisible();
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('link', { name: `需求 #${id}`, exact: true }).click();
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('operations.png'), fullPage: true });
  await page.getByRole('button', { name: '取消需求', exact: true }).focus();
  await expect(page.getByRole('button', { name: '取消需求', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: '确认取消' })).toContainText('关闭关联 PR');
  await page.getByRole('button', { name: '确认取消并关闭关联 PR' }).click();
  await expect(page.getByRole('status')).toContainText('操作已保存');
  const context = page.context();
  await page.close();
  await expect.poll(async () => {
    const response = await context.request.get(`/api/requirements/${id}/operations`);
    return (await response.json()).requirement.cleanup_complete;
  }).toBe(true);
  page = await context.newPage();
  await page.goto(`/requirements/${id}`);
  await expect(page.getByText('业务状态 Cancelled', { exact: false })).toBeVisible();
  await expect(page.getByText('取消收尾：已完成', { exact: false })).toBeVisible();
  await expect(page.getByRole('button', { name: '恢复', exact: true })).toHaveCount(0);
  await page.goto('/inbox');
  await expect(page.getByRole('heading', { name: '待办箱', exact: true })).toBeVisible();
  await expect(page.getByText('正在加载持久化状态…')).toHaveCount(0);
  await expect(page.getByRole('link', { name: `需求 #${id}`, exact: true })).toHaveCount(0);
  await page.goto('/requirements/list');
  await expect(page.getByRole('link', { name: `#${id} Persisted HTTP fixture` })).toBeVisible();
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('disconnect displays stale state and reconnect continues from the database', async ({
  page,
}) => {
  await page.goto('/inbox');
  await expect(page.getByRole('button', { name: '刷新状态' })).toBeVisible();
  await page.route('**/api/inbox', (route) => route.abort());
  await page.getByRole('button', { name: '刷新状态' }).click();
  await expect(page.getByRole('alert').first()).toContainText('当前内容可能陈旧');
  await page.unroute('**/api/inbox');
  await page.getByRole('button', { name: '刷新状态' }).click();
  await expect(page.getByText('当前内容可能陈旧', { exact: false })).toHaveCount(0);
});
