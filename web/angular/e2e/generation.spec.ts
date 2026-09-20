import { test, expect } from '@playwright/test';
import { reviewRealGroup } from './m1-real-flow';
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash, randomUUID } from 'node:crypto';

test('explicit generation replay, edit, delete, reorder, reopen and readable failure', async ({
  page,
  context,
  browser,
}, info) => {
  const headers = { origin: 'http://127.0.0.1:4300', 'x-codexsymphony-csrf': '1' };
  const repositories = await context.request.get('/api/multi/repository');
  const current = (await repositories.json()) as { repositories: unknown[] };
  if (!current.repositories.length) {
    const scenarios = JSON.parse(readFileSync('../../api/capture-scenarios.json', 'utf8')) as {
      id: string;
      body: Record<string, unknown>;
    }[];
    const body = scenarios.find((scenario) => scenario.id === 'configured')!.body;
    const configured = await context.request.put('/api/repository', {
      headers,
      data: { ...body, request_id: randomUUID(), version: 0 },
    });
    expect([200, 409]).toContain(configured.status());
  }
  const real = process.env['GH61_REAL_GENERATION'] === '1';
  let input: { label: string; text: string };
  let draftId = '';
  if (real) {
    test.setTimeout(180000);
    input = {
      label: `GH61 real browser ${info.project.name} ${randomUUID()}`,
      text:
        info.project.name === 'desktop'
          ? 'For registered repository ID 1, change the empty draft list message to Chinese 暂无草稿. One code_change child only. Scope: UI copy only. AC: empty list displays 暂无草稿, existing draft listing unchanged. Validate with a component test.'
          : 'For registered repository ID 1, add task tags. Split into at least three independent code_change items and one final validation_only item: (1) database tag persistence and API CRUD with schema tests, (2) desktop tag management with component tests depending on API, (3) mobile tag filtering with accessibility tests depending on desktop, (4) whole flow integration validation only depending on the three code changes. Parent ACs: CRUD persists across restart, desktop management, mobile filtering, integrated keyboard accessible flow. Scope excludes auth, notifications and deployment.',
    };
  } else {
    // A named protocol fixture for UI tests; actual model evidence is separate.
    const id = `browser-generation-${randomUUID()}`;
    input = {
      label: `scripted UI ${id}`,
      text: 'Synthetic UI generation fixture; not real model acceptance.',
    };
    const document = readFileSync('../../apps/server/tests/fixtures/draft-v1.json', 'utf8');
    const draft = await context.request.post('/api/drafts', {
      headers,
      data: { version: 0, source: { format: 'json', label: input.label, text: document } },
    });
    expect(draft.ok(), await draft.text()).toBeTruthy();
    draftId = ((await draft.json()) as { id: string }).id;
    const body = { draft_id: null, label: input.label, text: input.text, version: 0 };
    const hash = createHash('sha256').update(JSON.stringify(body)).digest('hex');
    const request = JSON.stringify({ ...body, request_id: id }).replaceAll("'", "''");
    execFileSync('psql', [
      process.env['TEST_DATABASE_URL']!,
      '-X',
      '-v',
      'ON_ERROR_STOP=1',
      '-c',
      `INSERT INTO draft_generation(id,request,fingerprint,draft_id,input_version,output_version,status,usage,limits) VALUES('${id}','${request}','${hash}','${draftId}',0,1,'succeeded','{"input":null,"cached":null,"output":null,"model_seconds":null,"complete":false}','{"tokens":30000,"turns":1,"model_seconds":120}')`,
    ]);
  }
  await page.goto('/drafts');
  await page.getByLabel('生成来源说明', { exact: true }).fill(input.label);
  await page.getByLabel('用自然语言描述需求', { exact: true }).fill(input.text);
  await page.getByRole('button', { name: '生成草稿', exact: true }).focus();
  const response = page.waitForResponse(
    (r) => r.request().method() === 'POST' && r.url().endsWith('/api/draft-generations'),
  );
  await page.keyboard.press('Enter');
  const submitted = await response;
  expect(submitted.status()).toBe(200);
  let record = (await submitted.json()) as {
    id: string;
    status: string;
    draft_id: string;
    usage: { input: number; output: number };
  };
  draftId = record.draft_id;
  for (let attempt = 0; record.status === 'running' && attempt < 65; attempt++) {
    await page.waitForTimeout(2000);
    await page.getByRole('button', { name: '刷新生成记录', exact: true }).click();
    record = (await (
      await context.request.get(`/api/draft-generations/${record.id}`)
    ).json()) as typeof record;
  }
  if (real) writeFileSync(info.outputPath('real-generation.json'), JSON.stringify(record, null, 2));
  expect(record.status).toBe('succeeded');
  if (real) {
    expect(record.usage.input).toBeGreaterThan(0);
    expect(record.usage.output).toBeGreaterThan(0);
  }
  const row = page.getByRole('listitem').filter({ hasText: record.id });
  await expect(row).toContainText('草稿已保存，未授权');
  await row.getByRole('button', { name: '打开生成结果（替换编辑区）', exact: true }).click();
  const text = page.getByLabel('草稿原文', { exact: true });
  await expect(text).not.toHaveValue('');
  const document = JSON.parse(await text.inputValue()) as {
    parent: { goal: string };
    children: { id: string; kind: string; order: number; depends_on: string[] }[];
  };
  if (real) {
    writeFileSync(
      info.outputPath('real-generated-document.json'),
      JSON.stringify(document, null, 2),
    );
    if (info.project.name === 'desktop') expect(document.children.length).toBe(1);
    else {
      expect(
        document.children.filter((child) => child.kind === 'code_change').length,
      ).toBeGreaterThanOrEqual(3);
      expect(
        document.children.filter((child) => child.kind === 'validation_only').length,
      ).toBeGreaterThanOrEqual(1);
    }
  }
  const ids = document.children.map((child) => child.id);
  document.parent.goal = `Edited generated ${info.project.name}`;
  if (!real && document.children.length > 1) document.children.pop();
  if (!real)
    document.children.reverse().forEach((child, index) => {
      child.order = index + 1;
      child.depends_on = [];
    });
  await text.fill(JSON.stringify(document));
  await page.getByRole('button', { name: '保存父子 Draft', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Draft 已保存');
  await page.reload();
  await page.getByRole('button', { name: `${document.parent.goal} · Draft`, exact: false }).click();
  await expect(text).toHaveValue(JSON.stringify(document));
  expect(document.children.every((child) => ids.includes(child.id))).toBe(true);
  const reread = await context.request.get(`/api/drafts/${draftId}`);
  const saved = (await reread.json()) as { document: unknown };
  expect(saved.document).toEqual(document);
  if (real) writeFileSync(info.outputPath('real-saved-draft.json'), JSON.stringify(saved, null, 2));
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('generated-edited.png'), fullPage: true });
  if (real && info.project.name === 'mobile') await reviewRealGroup(page, browser, draftId, info);
  await text.fill('{invalid');
  await page.getByRole('button', { name: '保存父子 Draft', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('invalid draft JSON');
  await expect(text).toHaveValue('{invalid');
});
