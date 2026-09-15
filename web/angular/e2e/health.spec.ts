import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

test('connects to the real API and is accessible on this viewport', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('服务状态');
  await expect(page.getByRole('status')).toContainText('连接正常');
  await expect(page.getByRole('button', { name: '重新检查' })).toBeEnabled();
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(results.violations).toEqual([]);
});

test('recovers after a temporary connection failure', async ({ page }) => {
  await page.route('**/api/health', (route) => route.abort('connectionfailed'));
  await page.goto('/');
  await expect(page.getByRole('status')).toContainText('暂时无法连接');
  await page.unroute('**/api/health');
  await page.getByRole('button', { name: '重新检查' }).click();
  await expect(page.getByRole('status')).toContainText('连接正常');
});
