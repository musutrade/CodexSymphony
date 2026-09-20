import { expect, type Browser, type Page, type TestInfo } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { writeFileSync } from 'node:fs';
import type { GroupView } from '../src/app/group-review-model';

// Invoked only by the explicitly enabled real-model acceptance run.
export async function reviewRealGroup(page: Page, browser: Browser, id: string, info: TestInfo) {
  await page.goto(`/drafts/${id}/review`);
  const response = await page.request.get(`/api/drafts/${id}/review`);
  const view = (await response.json()) as GroupView;
  const last = [...view.document.children].sort((a, b) => a.order - b.order).at(-1)!;
  expect(last.kind).toBe('validation_only');
  await page
    .getByLabel('覆盖语义评审结论', { exact: true })
    .fill(
      'Disposable M1 acceptance: final integration verifies the parent task-tag flow after database, desktop and mobile changes; no production execution authorized.',
    );
  for (const child of view.document.children) {
    await page
      .getByLabel(`${child.id} 允许修复范围`, { exact: true })
      .fill('Only the reviewed task-tag scope and its tests.');
    await page
      .getByLabel(`${child.id} 前置合并基线上的独立验证与安全合并评审`, { exact: true })
      .fill(
        'Validate each layer on the merged dependency baseline before dependent UI work; final item validates the whole flow.',
      );
    for (const ac of child.acceptance_criteria) {
      await page
        .getByLabel(`${child.id} ${ac.id} 测试选择器`, { exact: true })
        .fill('m1_task_tags');
    }
  }
  await page.getByRole('button', { name: '保存评审版本', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('评审版本已保存');
  await page.getByRole('button', { name: '一次确认整组授权', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  expect(
    ((await (await page.request.get(`/api/drafts/${id}/review`)).json()) as GroupView).queue,
  ).toBeNull();
  for (const [index, ac] of view.document.parent.acceptance_criteria.entries()) {
    await page.getByRole('button', { name: `添加 ${ac.id} 覆盖映射`, exact: true }).click();
    await page
      .getByRole('combobox', { name: `映射 ${index + 1} 子项 ID`, exact: true })
      .selectOption(last.id);
    const verification = /keyboard|键盘/i.test(ac.description)
      ? last.acceptance_criteria.findIndex((item) => /keyboard|键盘/i.test(item.description))
      : 0;
    expect(verification).toBeGreaterThanOrEqual(0);
    await page
      .getByLabel(`映射 ${index + 1} 子项 AC ID`, { exact: true })
      .fill(last.acceptance_criteria[verification].id);
    await page
      .getByLabel(`映射 ${index + 1} 验证步骤 ID`, { exact: true })
      .fill(`verify-${verification + 1}`);
  }
  await page.getByRole('button', { name: '保存评审版本', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('评审版本已保存');
  await page.getByRole('button', { name: '一次确认整组授权', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('依赖队列');
  const authorized = (await (
    await page.request.get(`/api/drafts/${id}/review`)
  ).json()) as GroupView;
  expect(authorized.authorizations).toHaveLength(1);
  expect(authorized.business_complete).toBe(false);
  expect(authorized.execution?.completed).toBe(0);
  expect(
    authorized.execution?.items.find((item) => item.child_id === last.id)?.requirement_id,
  ).toBeNull();
  writeFileSync(info.outputPath('m1-authorized.json'), JSON.stringify(authorized, null, 2));
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('m1-mobile-queue.png'), fullPage: true });
  await page.getByRole('heading', { name: '组依赖队列' }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: info.outputPath('m1-mobile-queue-viewport.png') });
  const desktop = await browser.newContext({
    baseURL: 'http://127.0.0.1:4300',
    viewport: { width: 1440, height: 1000 },
  });
  try {
    const other = await desktop.newPage();
    await other.goto(`/drafts/${id}/review`);
    await expect(other.getByRole('heading', { name: '组依赖队列' })).toBeVisible();
    const readback = (await (
      await desktop.request.get(`/api/drafts/${id}/review`)
    ).json()) as GroupView;
    expect(readback.document).toEqual(authorized.document);
    expect(readback.authorizations).toEqual(authorized.authorizations);
    expect(readback.execution?.items).toEqual(authorized.execution?.items);
    expect((await new AxeBuilder({ page: other }).analyze()).violations).toEqual([]);
    expect(await other.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await other.screenshot({ path: info.outputPath('m1-desktop-same-queue.png'), fullPage: true });
    writeFileSync(info.outputPath('m1-desktop-readback.json'), JSON.stringify(readback, null, 2));
  } finally {
    await desktop.close();
  }
  await page.goto('/drafts');
  await page
    .getByRole('button', { name: `${view.document.parent.goal} · Draft`, exact: false })
    .click();
}
