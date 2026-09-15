import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

test('create, edit, review Ready, reload and withdraw a durable requirement', async ({
  page,
}, testInfo) => {
  await page.goto('/requirements');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('需求工作台');
  await expect(page.getByRole('button', { name: '刷新列表', exact: true })).toBeEnabled();
  if (await page.getByRole('heading', { name: '登记首个可信仓库' }).isVisible()) {
    await page.getByLabel('项目名称', { exact: true }).fill('Disposable');
    await page
      .getByLabel('GitHub 仓库（owner/repository）', { exact: true })
      .fill('musutrade/disposable');
    await page.getByLabel('GitHub 仓库数字 ID', { exact: true }).fill('123');
    const [configured] = await Promise.all([
      page.waitForResponse(response => response.request().method() === 'PUT' && response.url().endsWith('/api/repository')),
      page.getByRole('button', { name: '确认仓库与初始策略' }).click(),
    ]);
    // Desktop and mobile share the synthetic fixture; the losing CAS refreshes.
    if (configured.status() === 409) {
      await expect(page.getByRole('button', { name: '刷新列表', exact: true })).toBeEnabled();
      await page.getByRole('button', { name: '刷新列表', exact: true }).click();
    }
    await expect(page.getByRole('heading', { name: '当前仓库策略' })).toBeVisible();
  }
  await page.getByRole('button', { name: '保存 Draft', exact: true }).click();
  const title = page.getByLabel('标题', { exact: true });
  await expect(title).toHaveAttribute('aria-describedby', /mat-mdc-error/);
  await title.fill(`Browser ${testInfo.project.name} ${Date.now()}`);
  await page
    .getByLabel('需求描述', { exact: true })
    .fill('Add a future test and preserve reviewed input.');
  await page.getByLabel('验收条件 1 描述', { exact: true }).fill('Future test passes');
  await page.getByLabel('步骤 1 测试选择器', { exact: true }).fill('future_test::passes');
  await page
    .getByLabel('步骤 1 预期结果', { exact: true })
    .fill('Exit 0 with all assertions passing');
  await page.getByLabel('联网意图（可选，逗号分隔）', { exact: true }).focus();
  await page.keyboard.press('Tab');
  expect(
    await page
      .getByRole('button', { name: '保存 Draft', exact: true })
      .evaluate((el) => getComputedStyle(el).outlineStyle),
  ).toBe('solid');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('Draft 已保存');
  await title.fill(`Edited ${testInfo.project.name} ${Date.now()}`);
  const savedTitle = await title.inputValue();
  await page.getByRole('button', { name: '保存 Draft', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Draft 已保存');
  await page.getByRole('button', { name: '评审已保存版本', exact: true }).click();
  await expect(page.getByRole('heading', { name: `确认评审：${savedTitle}` })).toBeVisible();
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('review.png'), fullPage: true });
  await page.getByRole('button', { name: '确认评审并 Ready', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('Ready 已持久化');
  await expect(title).toBeDisabled();
  await page.reload();
  await page.getByRole('button', { name: new RegExp(savedTitle), exact: false }).click();
  await expect(page.getByRole('button', { name: '撤回 Draft', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '撤回 Draft', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('已撤回 Draft');
  await expect(title).toBeEnabled();
  await expect(page.getByText(/冻结修订 1/)).toBeVisible();
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('withdrawn.png'), fullPage: true });
});

test('shows loading, failure and empty states, and blocks duplicate submission', async ({
  page,
}) => {
  await page.route('**/api/requirements', (route) => route.abort());
  await page.goto('/requirements');
  await expect(page.getByRole('alert')).toBeVisible();
  await page.unroute('**/api/requirements');
  await page.route('**/api/requirements', (route) => route.fulfill({ json: { requirements: [] } }));
  await page.getByRole('button', { name: '刷新列表', exact: true }).click();
  await expect(page.getByText('暂无需求。填写下方表单创建第一条需求。')).toBeVisible();
  let release: () => void = () => {};
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.unroute('**/api/requirements');
  await page.route('**/api/requirements', async (route) => {
    await pending;
    await route.fulfill({ json: { requirements: [] } });
  });
  await page.getByRole('button', { name: '刷新列表', exact: true }).click();
  await expect(page.getByRole('status')).toContainText('正在加载');
  await expect(page.getByRole('button', { name: '刷新列表', exact: true })).toBeDisabled();
  release();
  await expect(page.getByRole('button', { name: '刷新列表', exact: true })).toBeEnabled();
});
