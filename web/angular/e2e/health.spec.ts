import { test, expect } from './auth-fixture';
import AxeBuilder from '@axe-core/playwright';

test('connects to the real API and is accessible on this viewport', async ({ page }, testInfo) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('服务状态');
  await expect(page.getByRole('status')).toContainText('连接正常');
  await expect(page.getByRole('button', { name: '重新检查' })).toBeEnabled();
  const results = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
  expect(results.violations).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const main = page.getByRole('main');
  const layout = await main.evaluate((element) => ({
    width: element.getBoundingClientRect().width,
    padding: getComputedStyle(element).paddingLeft,
  }));
  expect(layout.width).toBeLessThanOrEqual(1200);
  expect(layout.padding).toBe(testInfo.project.name === 'mobile' ? '16px' : '24px');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: '跳到主要内容' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(main).toBeFocused();
  await page.keyboard.press('Tab');
  const retry = page.getByRole('button', { name: '重新检查' });
  await expect(retry).toBeFocused();
  expect(await retry.evaluate((element) => getComputedStyle(element).outlineStyle)).toBe('solid');
  const bounds = await retry.boundingBox();
  expect(bounds?.height).toBeGreaterThanOrEqual(40);
  await page.screenshot({ path: testInfo.outputPath('healthy.png'), fullPage: true });
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('连接正常');
});

test('recovers after a temporary connection failure', async ({ page }) => {
  await page.route('**/api/health', (route) => route.abort('connectionfailed'));
  await page.goto('/');
  await expect(page.getByRole('status')).toContainText('暂时无法连接');
  await page.unroute('**/api/health');
  await page.getByRole('button', { name: '重新检查' }).click();
  await expect(page.getByRole('status')).toContainText('连接正常');
});

test('shows pending and empty responses, then retries with reduced motion and forced colors', async ({ page }, testInfo) => {
  let release: () => void = () => {};
  const pending = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/api/health', async (route) => {
    await pending;
    await route.fulfill({ status: 204 });
  });
  await page.goto('/');
  await expect(page.getByRole('status')).toContainText('正在检查连接');
  await expect(page.getByRole('button', { name: '重新检查' })).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath('loading.png'), fullPage: true });
  release();
  await expect(page.getByRole('status')).toContainText('暂时无法连接');
  await expect(page.getByRole('button', { name: '重新检查' })).toBeEnabled();
  expect((await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('empty-response.png'), fullPage: true });
  await page.emulateMedia({ reducedMotion: 'reduce', forcedColors: 'active' });
  await page.unroute('**/api/health');
  await page.getByRole('button', { name: '重新检查' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status')).toContainText('连接正常');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath('forced-colors.png'), fullPage: true });
});
