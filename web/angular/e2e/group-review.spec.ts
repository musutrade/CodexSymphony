import { test, expect, headers as authHeaders } from './auth-fixture';
import AxeBuilder from '@axe-core/playwright';
import { readFileSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { initialReview } from '../src/app/group-review-model';
import type { GroupView } from '../src/app/group-review-model';

test('reviews three code items plus integration, rejects missing coverage and authorizes atomically', async ({
  page,
  context,
}, info) => {
  const headers = await authHeaders(context);
  const scenarios = JSON.parse(readFileSync('../../api/capture-scenarios.json', 'utf8')) as {
    id: string;
    body: Record<string, unknown>;
  }[];
  const initial = scenarios.find((s) => s.id === 'configured')!.body;
  const configured = await context.request.put('/api/repository', {
    headers,
    data: { ...initial, request_id: randomUUID() },
  });
  expect([200, 409]).toContain(configured.status());
  const repository = {
    project: 'GH62 synthetic',
    remote: `test/group-${randomUUID()}`,
    github_repository_id: Date.now() * 100 + (info.project.name === 'desktop' ? 1 : 2),
    base_branch: 'main',
    revoked: false,
    reason: 'Disposable group browser fixture',
    policy: {
      allowed_checks: ['cargo_test'],
      max_timeout_seconds: 120,
      token_limit: 1000,
      turn_limit: 10,
      model_work_seconds: 600,
      gate_recovery_policy: 'one_code_repair',
    },
  };
  const registered = await context.request.put('/api/multi/repository', {
    headers,
    data: {
      request_id: randomUUID(),
      version: 0,
      repository_id: 10000 + Math.floor(Math.random() * 100000000),
      repository,
    },
  });
  expect(registered.ok(), await registered.text()).toBeTruthy();
  const savedRepository = (await registered.json()) as { id: number };
  const document = JSON.parse(
    readFileSync('../../apps/server/tests/fixtures/group-draft.json', 'utf8'),
  ) as { parent: { goal: string }; children: { repository_id: number }[] };
  document.parent.goal = `GH62 ${info.project.name} ${randomUUID()}`;
  document.children.forEach((child) => (child.repository_id = savedRepository.id));
  const imported = await context.request.post('/api/drafts', {
    headers,
    data: {
      version: 0,
      source: { format: 'json', label: 'synthetic group UI', text: JSON.stringify(document) },
    },
  });
  expect(imported.ok(), await imported.text()).toBeTruthy();
  const draft = (await imported.json()) as { id: string };
  await page.goto(`/drafts/${draft.id}/review`);
  await expect(page.getByRole('heading', { name: '整组评审与授权' })).toBeVisible();
  await expect(page.getByRole('button', { name: '保存评审版本', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '保存评审版本', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('评审版本已保存');
  await page.getByRole('button', { name: '一次确认整组授权', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('semantic');
  const before = await context.request.get(`/api/drafts/${draft.id}/review`);
  expect(((await before.json()) as GroupView).queue).toBeNull();
  await page.getByRole('button', { name: '添加 P-AC1 覆盖映射', exact: true }).click();
  await page.getByRole('combobox', { name: '映射 1 子项 ID', exact: true }).selectOption('C4');
  await page.getByLabel('映射 1 子项 AC ID', { exact: true }).fill('AC1');
  await page.getByLabel('映射 1 验证步骤 ID', { exact: true }).fill('verify-1');
  await page
    .getByLabel('覆盖语义评审结论', { exact: true })
    .fill(
      'Integrated flow is validated by C4 after the three independently safe code items merge.',
    );
  for (const id of ['C1', 'C2', 'C3', 'C4']) {
    await page
      .getByLabel(`${id} 允许修复范围`, { exact: true })
      .fill('Only this item AC in the approved repository.');
    await page
      .getByLabel(`${id} 前置合并基线上的独立验证与安全合并评审`, { exact: true })
      .fill('Run regression suite on main containing merged dependencies; no later fix required.');
    await page.getByLabel(`${id} AC1 测试选择器`, { exact: true }).fill('group_review');
    await page.getByLabel(`${id} token 额度`, { exact: true }).fill('100');
  }
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('group-review.png'), fullPage: true });
  await page.getByRole('heading', { name: '整组评审与授权' }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: info.outputPath('review-top.png') });
  // Keyboard completes both save and confirmation without pointer-only controls.
  await page.getByRole('button', { name: '保存评审版本', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('评审版本已保存');
  await page.getByRole('button', { name: '一次确认整组授权', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('依赖队列');
  await page.reload();
  await expect(page.getByText('状态：已授权 / 依赖队列', { exact: false })).toBeVisible();
  const reread = await context.request.get(`/api/drafts/${draft.id}/review`);
  const result = (await reread.json()) as GroupView;
  writeFileSync(info.outputPath('group-view.json'), JSON.stringify(result, null, 2));
  expect(result.authorizations).toHaveLength(1);
  expect(result.business_complete).toBe(false);
  await expect(page.getByRole('heading', { name: '组依赖队列' })).toBeVisible();
  await expect(
    page.getByText('等待评审精确版本验证配置与授权仓库'),
  ).toBeVisible();
  expect(result.execution?.owner).toBeNull();
  expect(result.execution?.completed).toBe(0);
  expect(result.execution?.items.map((i) => i.order)).toEqual([1, 2, 3, 4]);
  expect(result.authorizations[0].snapshot.review.items).toHaveLength(4);
  expect(result.authorizations[0].snapshot.group_budget.tokens).toBe(400);
  const bypass = await context.request.post(`/api/requirements/${draft.id}/ready`, {
    headers,
    data: {},
  });
  expect(bypass.status()).toBe(409);
  await page.emulateMedia({ reducedMotion: 'reduce', forcedColors: 'active' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: info.outputPath('group-authorized.png'), fullPage: true });
});

test('edits and reorders the persisted unstarted queue with difference review', async ({
  page,
  context,
}, info) => {
  const headers = await authHeaders(context);
  const scenarios = JSON.parse(readFileSync('../../api/capture-scenarios.json', 'utf8')) as {
    id: string;
    body: Record<string, unknown>;
  }[];
  const initial = scenarios.find((s) => s.id === 'configured')!.body;
  const configured = await context.request.put('/api/repository', {
    headers,
    data: { ...initial, request_id: randomUUID() },
  });
  expect([200, 409]).toContain(configured.status());
  const repository = {
    project: 'GH62 synthetic',
    remote: `test/group-${randomUUID()}`,
    github_repository_id: Date.now() * 100 + (info.project.name === 'desktop' ? 1 : 2),
    base_branch: 'main',
    revoked: false,
    reason: 'Disposable group browser fixture',
    policy: {
      allowed_checks: ['cargo_test'],
      max_timeout_seconds: 120,
      token_limit: 1000,
      turn_limit: 10,
      model_work_seconds: 600,
      gate_recovery_policy: 'one_code_repair',
    },
  };
  const registered = await context.request.put('/api/multi/repository', {
    headers,
    data: {
      request_id: randomUUID(),
      version: 0,
      repository_id: 10000 + Math.floor(Math.random() * 100000000),
      repository,
    },
  });
  expect(registered.ok(), await registered.text()).toBeTruthy();
  const savedRepository = (await registered.json()) as { id: number };
  const document = JSON.parse(
    readFileSync('../../apps/server/tests/fixtures/group-draft.json', 'utf8'),
  ) as { parent: { goal: string }; children: { repository_id: number }[] };
  document.parent.goal = `GH62 ${info.project.name} ${randomUUID()}`;
  document.children.forEach((child) => (child.repository_id = savedRepository.id));
  const imported = await context.request.post('/api/drafts', {
    headers,
    data: {
      version: 0,
      source: { format: 'json', label: 'synthetic group UI', text: JSON.stringify(document) },
    },
  });
  expect(imported.ok(), await imported.text()).toBeTruthy();
  const draft = (await imported.json()) as { id: string };
  const initialView = (await (
    await context.request.get(`/api/drafts/${draft.id}/review`)
  ).json()) as GroupView;
  const review = initialReview(initialView);
  review.semantic_review = 'Synthetic reviewed integration coverage';
  review.coverage = [
    { parent_ac: 'P-AC1', child_id: 'C4', child_revision: 1, child_ac: 'AC1', step_id: 'verify-1' },
  ];
  review.items.forEach((item) => {
    item.repair_scope = 'Only reviewed AC';
    item.merged_baseline_review = 'Independently safe on merged baseline';
    item.verification[0].step.selector = 'group_review';
  });
  expect(
    (
      await context.request.put(`/api/drafts/${draft.id}/review`, {
        headers,
        data: { version: 0, draft_revision: 1, review },
      })
    ).status(),
  ).toBe(200);
  expect(
    (
      await context.request.post(`/api/drafts/${draft.id}/authorize`, {
        headers,
        data: { request_id: randomUUID(), version: 1, draft_revision: 1 },
      })
    ).status(),
  ).toBe(200);
  await page.goto(`/drafts/${draft.id}/review`);
  const editor = page.getByRole('region', { name: '编辑未开始队列' });
  await editor.getByRole('button', { name: '上移 C2', exact: true }).click();
  await expect(editor.getByRole('alert')).toContainText('dependencies');
  await editor.getByRole('button', { name: '编辑变化内容与评审', exact: true }).click();
  const updated = JSON.parse(
    await editor.getByLabel('父子需求内容（JSON）').inputValue(),
  ) as GroupView['document'];
  updated.children[1].goal = 'Reviewed changed second item';
  updated.children[2].depends_on = ['C1'];
  updated.children[3].depends_on = ['C2', 'C3'];
  await editor.getByLabel('父子需求内容（JSON）').fill(JSON.stringify(updated));
  await editor.getByRole('button', { name: '保存差异并冻结受影响项' }).click();
  await expect(editor.getByRole('heading', { name: /待评审变化/ })).toBeVisible();
  await page.reload();
  await expect(editor.getByText('冻结项及依赖：C2、C3、C4', { exact: false })).toBeVisible();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: info.outputPath('queue-difference.png'), fullPage: true });
  await editor.getByRole('button', { name: '确认以上变化版本与策略并重新授权' }).focus();
  await page.keyboard.press('Enter');
  await expect(editor.getByRole('heading', { name: /待评审变化/ })).toHaveCount(0);
  await editor.getByRole('button', { name: '上移 C3', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(editor.getByText('2. C3', { exact: false })).toBeVisible();
  await page.reload();
  await expect(editor.getByText('2. C3', { exact: false })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('queue-reordered.png'), fullPage: true });
  await page.emulateMedia({ reducedMotion: 'reduce', forcedColors: 'active' });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
