// Explicitly invoked real single-repository smoke; never part of default tests.
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { resumeEvidence, observeDelivery } from './a01-observe.mjs';
const require = createRequire(new URL('../web/angular/package.json', import.meta.url));
const [inputPath, outputPath, option, previousPath] = process.argv.slice(2);
if (option && (option !== '--resume' || !previousPath)) throw new Error('expected --resume PREVIOUS_RESULT.json');
if (!inputPath || !outputPath) throw new Error('usage: node tools/a01-smoke.mjs INPUT.json NEW_EVIDENCE_DIRECTORY [--resume PREVIOUS_RESULT.json]');
const input = JSON.parse(await readFile(inputPath, 'utf8'));
const origin = new URL(input.origin);
if (!['localhost', '127.0.0.1', '[::1]'].includes(origin.hostname) || origin.protocol !== 'http:') throw new Error('localhost product origin required');
if (input.authorized_repository !== input.repository || !Number.isInteger(input.repository_id) || !input.title || !input.description || !input.acceptance || !input.selector || !input.expected) throw new Error('explicit designated repository and complete smoke requirement required');
const directory = resolve(outputPath);
await mkdir(directory); // Do not overwrite or silently start another attempt.
const evidence = { schema: 'codexsymphony-a01/v1', mode: 'real', status: 'started', started_at: new Date().toISOString(), repository: input.repository, repository_id: input.repository_id, requirement_id: null, revision: null, delivery: null };
const save = () => writeFile(resolve(directory, 'result.json'), JSON.stringify(evidence, null, 2) + '\n');
await save();
let browser;
try {
  if (previousPath) {
    const previousBytes = await readFile(previousPath);
    const previous = JSON.parse(previousBytes.toString('utf8'));
    Object.assign(evidence, resumeEvidence(previous, input), {
      resumed_from: resolve(previousPath),
      resumed_from_sha256: createHash('sha256').update(previousBytes).digest('hex'),
    });
    await save();
  } else {
    const { chromium, expect } = require('@playwright/test');
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto(new URL('/requirements', origin).href);
    const repositoryResponse = await context.request.get(new URL('/api/repository', origin).href);
    if (!repositoryResponse.ok()) throw new Error('repository read failed');
    const registered = await repositoryResponse.json();
    if (!registered.repositories?.some(r => r.repository.remote === input.repository && r.repository.github_repository_id === input.repository_id && !r.repository.revoked)) throw new Error('designated repository is not configured');
    await page.getByLabel('标题', { exact: true }).fill(input.title);
    await page.getByLabel('需求描述', { exact: true }).fill(input.description);
    await page.getByLabel('验收条件 1 描述', { exact: true }).fill(input.acceptance);
    await page.getByLabel('步骤 1 测试选择器', { exact: true }).fill(input.selector);
    await page.getByLabel('步骤 1 预期结果', { exact: true }).fill(input.expected);
    const [draft] = await Promise.all([
      page.waitForResponse(r => r.request().method() === 'POST' && r.url().endsWith('/api/requirements')),
      page.getByRole('button', { name: '保存 Draft', exact: true }).click(),
    ]);
    if (draft.status() !== 201) throw new Error(`Draft returned ${draft.status()}`);
    const requirement = await draft.json();
    evidence.requirement_id = requirement.id;
    await save();
    await page.getByRole('button', { name: '评审已保存版本', exact: true }).click();
    await page.screenshot({ path: resolve(directory, 'review.png'), fullPage: true });
    const [ready] = await Promise.all([
      page.waitForResponse(r => r.url().endsWith(`/api/requirements/${requirement.id}/ready`)),
      page.getByRole('button', { name: '确认评审并 Ready', exact: true }).click(),
    ]);
    if (!ready.ok()) throw new Error(`Ready returned ${ready.status()}`);
    evidence.ready = await ready.json();
    evidence.revision = evidence.ready.revision;
    await save();
    await expect(page.getByRole('status')).toContainText('Ready 已持久化');
    await context.setOffline(true);
    await page.close(); // No browser session may be needed to drive the backend.
    await context.close();
  }
  await observeDelivery(input, evidence, save);
} catch (error) {
  evidence.status = 'incomplete';
  evidence.error = String(error);
  process.exitCode = 1;
} finally {
  evidence.finished_at = new Date().toISOString();
  await save();
  await browser?.close();
}
