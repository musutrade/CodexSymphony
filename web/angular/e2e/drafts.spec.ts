import { test, expect, headers as authHeaders } from './auth-fixture';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';

test('imports deterministic JSON and Markdown, persists edits and shows accessible parent/child drafts', async ({
  page,
  context,
}, info) => {
  const headers = await authHeaders(context);
  const repositories = await context.request.get('/api/multi/repository');
  const current = (await repositories.json()) as { repositories: unknown[] };
  if (!current.repositories.length) {
    const scenarios = JSON.parse(readFileSync('../../api/capture-scenarios.json', 'utf8')) as {
      id: string;
      body: Record<string, unknown>;
    }[];
    const body = scenarios.find((s) => s.id === 'configured')!.body;
    const result = await context.request.put('/api/repository', {
      headers,
      data: { ...body, request_id: crypto.randomUUID(), version: 0 },
    });
    expect([200, 409]).toContain(result.status());
  }
  await page.goto('/requirements');
  await page.getByRole('link', { name: '导入与查看父子草稿' }).click();
  const text = readFileSync('../../apps/server/tests/fixtures/draft-v1.json', 'utf8');
  for (const format of ['json', 'markdown']) {
    await page.getByRole('button', { name: '新建导入草稿', exact: true }).click();
    await page.getByLabel('输入格式', { exact: true }).selectOption(format);
    const source =
      format === 'json'
        ? text
        : readFileSync('../../apps/server/tests/fixtures/draft-v1.md', 'utf8');
    await page
      .getByLabel('来源说明', { exact: true })
      .fill(`Browser ${info.project.name} ${format}`);
    await page.getByLabel('草稿原文', { exact: true }).fill(source);
    const response = page.waitForResponse(
      (r) => r.request().method() === 'POST' && r.url().endsWith('/api/drafts'),
    );
    await page.getByRole('button', { name: '保存父子 Draft', exact: true }).focus();
    await page.keyboard.press('Enter');
    const result = await response;
    expect(result.status()).toBe(200);
    await expect(page.getByRole('status')).toContainText('未授权执行');
    const link = await page
      .getByRole('link', { name: '评审覆盖、策略与整组预算' })
      .getAttribute('href');
    const id = link!.split('/')[2];
    const persisted = await context.request.get(`/api/drafts/${id}`);
    expect(persisted.status()).toBe(200);
    const saved = (await persisted.json()) as {
      id: string;
      document: unknown;
      source: { text: string };
    };
    expect(saved.document).toEqual(JSON.parse(text));
    expect(saved.source.text).toBe(source);
    await expect(page.getByRole('status')).toContainText('未授权执行');
    const child = page.getByRole('article', { name: '子项 C2' });
    await expect(child).toContainText('validation_only');
    await expect(child).toContainText('依赖：C1');
    await expect(page.getByRole('heading', { name: '父目标 · P1' })).toBeVisible();
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: info.outputPath(`${format}-draft.png`), fullPage: true });
    const blocked = await context.request.post(`/api/requirements/${saved.id}/ready`, {
      headers,
      data: { request_id: 'not-authorized', version: 1, repository_version: 1 },
    });
    expect(blocked.status()).toBe(409);
    const document = JSON.parse(text) as { parent: { goal: string } };
    document.parent.goal = `Edited ${info.project.name} ${format}`;
    await page.getByLabel('输入格式', { exact: true }).selectOption('json');
    await page.getByLabel('草稿原文', { exact: true }).fill(JSON.stringify(document));
    await page.getByRole('button', { name: '保存父子 Draft', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('版本 2');
    await page.reload();
    await page
      .getByRole('button', { name: `${document.parent.goal} · Draft · 版本 2`, exact: true })
      .click();
    await expect(page.getByRole('status')).toContainText('已读回 Draft 版本 2');
    await page.getByLabel('草稿原文', { exact: true }).fill('invalid JSON');
    await page.getByRole('button', { name: '保存父子 Draft', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('invalid draft JSON');
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  }
});
