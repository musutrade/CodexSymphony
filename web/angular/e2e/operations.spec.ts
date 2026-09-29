import { test, expect, headers as authHeaders } from './auth-fixture';
import AxeBuilder from '@axe-core/playwright';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

// Ordinary project fixtures in the disposable database, never a production endpoint.
function fixture(id: number, suffix: string) {
  const url = process.env['TEST_DATABASE_URL'];
  if (!url) throw new Error('TEST_DATABASE_URL is required for persisted operator fixtures');
  const sql = readFileSync('../../api/capture-fixture.sql', 'utf8')
    .replaceAll('900001', String(id))
    // The retained pre-merge history is a second Requirement in the same fixture.
    .replaceAll('900002', String(id * 10))
    .replaceAll('capture-', `browser-${suffix}-`);
  const scenarios = JSON.parse(readFileSync('../../api/capture-scenarios.json', 'utf8')) as {
    id: string;
    body?: { repository: unknown };
  }[];
  const repository = scenarios.find((scenario) => scenario.id === 'configured')?.body?.repository;
  if (!repository) throw new Error('synthetic repository fixture missing');
  execFileSync(
    'psql',
    [
      url,
      '-X',
      '-v',
      'ON_ERROR_STOP=1',
      '--single-transaction',
      '--set',
      `repository_document=${JSON.stringify(repository)}`,
    ],
    {
      input:
        "INSERT INTO repository(id,version,document) VALUES(1,1,:'repository_document'::jsonb) ON CONFLICT(id) DO NOTHING;\n" +
        sql,
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  );
}

function persisted(statement: string) {
  return execFileSync(
    'psql',
    [process.env['TEST_DATABASE_URL']!, '-XAt', '-v', 'ON_ERROR_STOP=1'],
    {
      input: statement,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe'],
    },
  ).trim();
}

test('recovery decisions and bounded notification replay use persistent identities and budgets', async ({
  page,
}, info) => {
  const suffix = `recovery-${info.project.name}-${Date.now()}`;
  const id = Date.now() + (info.project.name === 'desktop' ? 1 : 2);
  const run = `browser-${suffix}-run`;
  const validation = `browser-${suffix}-validation`;
  const plugin = `browser-${suffix}-notifier`;
  fixture(id, suffix);
  const beforeControl = JSON.parse(
    persisted('SELECT row_to_json(c)::text FROM execution_control c WHERE id=1;'),
  ) as { requirement_id: number | null; paused: boolean };
  expect(beforeControl.requirement_id).toBeNull();
  // Seed a historical producer fault in the disposable DB. Reads and decisions
  // below use the real protected API; this is not a plugin execution proof.
  persisted(`
    UPDATE requirement_revision SET document=jsonb_build_object('repository_id',1,'repository_version',1,'repository',(SELECT document FROM repository WHERE id=1)) WHERE requirement_id=${id};
    INSERT INTO requirement_budget(requirement_id,limits) VALUES(${id},'{"tokens":1000,"turns":10,"model_seconds":100}');
    INSERT INTO model_call(run_id,turn_id,requirement_id,intent,reserved,usage) VALUES('${run}','unknown-fixture',${id},'{}','{"tokens":100,"turns":1,"model_seconds":10}','{"input":null,"cached":null,"output":null,"model_seconds":null,"complete":false}');
    INSERT INTO runtime_model_identity(run_id,frozen,actual,matched) VALUES('${run}',
      '{"capability_version":"fixture-v1","repository_id":1,"repository_version":1,"selection":{"config":{"provider":"fixture-provider","model":"reviewed-fixture-model","effort":"medium"},"reason":"Fixture child override"},"source":"child_override"}',
      '{"provider":"fixture-provider","model":"different-fixture-model","effort":"high"}',false);
    INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result)
    VALUES('${validation}',${id},1,'${run}',repeat('a',40),repeat('b',40),'{}','[]','source','source','entry','entry','validation','blocked');
    INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason)
    VALUES('${validation}:unsupported',${id},'${validation}','local',jsonb_build_object('candidate_sha',repeat('a',40),'phase','local','feedback',jsonb_build_object('check_id','source-risk','verdict','unsupported','fault',jsonb_build_object('class','unsupported','code','mapping_gap','message','Missing exact counters','owner','collector maintainer','scope',jsonb_build_array('source.rs'),'resume_condition','Reviewed implementation and exact counters'))),'${validation}','blocked','Native unsupported evidence retained');
    INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:${plugin}','all','{}',true);
    INSERT INTO notification_plugin VALUES('${plugin}','fixture_${id}',true);
    SELECT lifecycle_append(${id},1,'browser_notification','${plugin}','{"result":"fixture producer failure"}');
    UPDATE notification_delivery SET state='failed',attempts=3,last_result='retry_exhausted' WHERE plugin_id='${plugin}';
    SELECT plugin_scope_admit('agent:codex','${run}',${id},1,1);
  `);
  const event = Number(persisted(`SELECT id FROM lifecycle_event WHERE source_id='${plugin}';`));
  try {
    await page.goto(`/requirements/${id}`);
    const recovery = page.getByRole('region', { name: '扩展故障与恢复决定' });
    await expect(recovery).toContainText('collector maintainer');
    const fault = recovery.locator('article').filter({ hasText: validation });
    await expect(fault.getByRole('button', { name: '核对并提交恢复决定' })).toBeDisabled();
    const budget = page.getByRole('region', { name: '冻结输入与仓库作用域' });
    await expect(budget).toContainText('未知用量调用 1 个');
    const models = page.getByRole('region', { name: '冻结模型与实际用量' });
    await expect(models).toContainText('reviewed-fixture-model');
    await expect(models).toContainText('different-fixture-model');
    await expect(models).toContainText('身份核对：不匹配');
    await expect(models).toContainText('用量待结算，保留预留');
    const before = persisted(
      `SELECT row_to_json(b)::text FROM requirement_budget b WHERE requirement_id=${id};`,
    );
    persisted(
      `UPDATE requirement SET paused=false WHERE id=${id}; UPDATE execution_control SET requirement_id=${id},paused=false WHERE id=1;`,
    );
    await page.getByRole('button', { name: '刷新状态', exact: true }).click();
    await fault.getByRole('button', { name: '核对并提交恢复决定' }).click();
    await recovery.getByLabel('恢复理由').fill('Reviewed same candidate recovery');
    await recovery.getByLabel('部署批准的计划 SHA-256').fill('1'.repeat(64));
    await recovery
      .getByLabel('已满足的恢复条件')
      .fill('Reviewed implementation and exact counters');
    const sent = page.waitForRequest(
      (request) => request.url().endsWith('/extension-recovery') && request.method() === 'POST',
    );
    await recovery.getByRole('button', { name: '保存恢复决定' }).click();
    const decision = (await sent).postDataJSON();
    await expect(recovery.getByRole('status')).toContainText('尚未报告会话启动');
    const headers = await authHeaders(page.context());
    const replay = await page.request.post(`/api/requirements/${id}/extension-recovery`, {
      headers,
      data: decision,
    });
    expect(replay.status()).toBe(200);
    expect((await replay.json()).started).toBe(false);
    const conflict = await page.request.post(`/api/requirements/${id}/extension-recovery`, {
      headers,
      data: { ...decision, reason: 'Changed input under old identity' },
    });
    expect(conflict.status()).toBe(409);
    expect(
      persisted(
        `SELECT row_to_json(b)::text FROM requirement_budget b WHERE requirement_id=${id};`,
      ),
    ).toBe(before);
    expect(persisted(`SELECT count(*) FROM model_call WHERE requirement_id=${id};`)).toBe('1');
    const lifecycle = page.getByRole('region', { name: '生命周期与通知投递' });
    const eventCard = lifecycle.locator('details').filter({ hasText: `事件 ${event}` });
    await eventCard.locator('summary').click();
    await eventCard.getByRole('button', { name: `补投通知 ${plugin}` }).click();
    await expect(lifecycle.getByRole('status')).toContainText('有限补投已接受');
    const delivery = persisted(
      `SELECT json_build_object('attempts',attempts,'limit',attempt_limit)::text FROM notification_delivery WHERE plugin_id='${plugin}' AND event_id=${event};`,
    );
    expect(JSON.parse(delivery)).toEqual({ attempts: 3, limit: 6 });
    const duplicate = await page.request.post(`/api/requirements/${id}/notifications/replay`, {
      headers,
      data: { event_id: event, plugin_id: plugin },
    });
    expect((await duplicate.json()).accepted).toBe(false);
    persisted(
      `UPDATE plugin_scope SET enabled=false WHERE plugin_id='notification:${plugin}'; UPDATE notification_delivery SET state='failed' WHERE plugin_id='${plugin}' AND event_id=${event};`,
    );
    await lifecycle.getByRole('button', { name: '刷新生命周期与模型' }).click();
    await eventCard.getByRole('button', { name: `补投通知 ${plugin}` }).click();
    await expect(lifecycle.getByRole('status')).toContainText('补投未接受');
    expect(persisted(`SELECT count(*) FROM model_call WHERE requirement_id=${id};`)).toBe('1');
    expect(
      (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
        .violations,
    ).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: info.outputPath('recovery-lifecycle.png'), fullPage: true });
    await info.attach('recovery-identities.json', {
      body: JSON.stringify({
        id,
        validation,
        event,
        decision,
        delivery: JSON.parse(delivery),
        budget_unchanged: true,
        model_call_count: 1,
      }),
      contentType: 'application/json',
    });
  } finally {
    persisted(
      `UPDATE requirement SET paused=true WHERE id=${id}; UPDATE execution_control SET requirement_id=NULL WHERE requirement_id=${id}; UPDATE execution_control SET paused=${beforeControl.paused} WHERE id=1 AND requirement_id IS NULL; UPDATE notification_plugin SET enabled=false WHERE id='${plugin}';`,
    );
  }
});

test('cancellation keeps an unknown delivery occupied until a late merged observation is reconciled', async ({
  page,
}, info) => {
  const suffix = `merge-race-${info.project.name}-${Date.now()}`;
  const id = Date.now() + (info.project.name === 'desktop' ? 1 : 2);
  const validation = `browser-${suffix}-recovery-validation`;
  const key = `browser-${suffix}-delivery`;
  const branch = `fixture-${id}`;
  fixture(id, suffix);
  const beforeControl = JSON.parse(
    persisted('SELECT row_to_json(c)::text FROM execution_control c WHERE id=1;'),
  ) as { requirement_id: number | null; paused: boolean };
  expect(beforeControl.requirement_id).toBeNull();
  // Owned producer observations simulate the cancellation/merge race. The
  // actual cancellation API and reconciler run; no GitHub request is made.
  persisted(`
    INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr) VALUES(${id},1,'{}',1);
    INSERT INTO github_pr(repository_id,number,requirement_id,observation,last_synced_at,stale)
    VALUES(${id},1,${id},jsonb_build_object('merge','Unknown','head','fixture-candidate','head_ref','${branch}','base_ref','main','policy','{}'::jsonb),extract(epoch FROM now())::bigint,false);
    INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy,pr_number)
    VALUES('${key}','${validation}',${id},1,${id},'fixture/project','${branch}','main','fixture-candidate','{}','{}',1);
    INSERT INTO delivery_action(action_key,kind,state,attempts) VALUES('${key}','publish','unknown',1);
    INSERT INTO delivery_attempt(action_key,kind,ordinal,operation) VALUES('${key}','publish',1,'create');
    UPDATE execution_control SET requirement_id=${id},paused=true WHERE id=1;
  `);
  try {
    await page.goto(`/requirements/${id}`);
    await page.getByRole('button', { name: '取消需求', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('region', { name: '确认取消' })).toContainText(
      '已合并或已交付的版本作为历史事实保留',
    );
    await page.getByRole('button', { name: '确认取消并关闭关联 PR' }).click();
    await expect(page.getByRole('status')).toContainText('操作已保存');
    await expect(page.getByText('取消收尾：处理中，仍保留占用', { exact: false })).toBeVisible();
    expect(persisted(`SELECT requirement_id FROM execution_control WHERE id=1;`)).toBe(String(id));
    expect(persisted(`SELECT released FROM delivery WHERE action_key='${key}';`)).toBe('f');
    // Late observer evidence cannot revive a cancelled Requirement or imply Done.
    persisted(
      `UPDATE github_pr SET observation=jsonb_set(observation,'{merge}','"Merged"'),last_synced_at=extract(epoch FROM now())::bigint,stale=false WHERE repository_id=${id} AND number=1;`,
    );
    await expect
      .poll(() => persisted(`SELECT cleanup_complete FROM requirement WHERE id=${id};`))
      .toBe('t');
    await page.reload();
    await expect(page.getByText('业务状态 Cancelled', { exact: false })).toBeVisible();
    await expect(page.getByText('取消收尾：已完成', { exact: false })).toBeVisible();
    await expect(page.getByRole('button', { name: '恢复', exact: true })).toHaveCount(0);
    await expect(page.locator('pre').filter({ hasText: 'Merged' })).toBeVisible();
    expect(
      persisted(
        `SELECT observation->>'merge' FROM github_pr WHERE repository_id=${id} AND number=1;`,
      ),
    ).toBe('Merged');
    expect(
      persisted(
        `SELECT count(*) FROM delivery_attempt WHERE action_key='${key}' AND operation='close';`,
      ),
    ).toBe('0');
    expect(persisted(`SELECT requirement_id IS NULL FROM execution_control WHERE id=1;`)).toBe('t');
    expect(
      (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
        .violations,
    ).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: info.outputPath('cancel-late-merge.png'), fullPage: true });
  } finally {
    persisted(
      `UPDATE execution_control SET requirement_id=NULL WHERE requirement_id=${id}; UPDATE execution_control SET paused=${beforeControl.paused} WHERE id=1 AND requirement_id IS NULL; UPDATE delivery SET released=true WHERE action_key='${key}';`,
    );
  }
});

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
  expect(savedResponse.status()).toBe(200);
  await expect(page.getByRole('status')).toContainText('操作已保存');
  await expect(page.getByRole('button', { name: '保存回答', exact: true })).toHaveCount(0);
  await expect(page.getByText('暂停意图：已暂停', { exact: false })).toBeVisible();
  const stale = await page.request.post(`/api/operator/questions/${question}/answer`, {
    headers: await authHeaders(page.context()),
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
  await expect
    .poll(async () => {
      const response = await context.request.get(`/api/requirements/${id}/operations`);
      return (await response.json()).requirement.cleanup_complete;
    })
    .toBe(true);
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

test('views retained diagnostics and downloads the complete verified export on this viewport', async ({
  page,
}, info) => {
  const suffix = `diagnostics-${info.project.name}-${Date.now()}`;
  const id = Date.now() + (info.project.name === 'desktop' ? 1 : 2);
  fixture(id, suffix);
  const artifact = `browser-${suffix}-diagnostic`;
  execFileSync('psql', [process.env['TEST_DATABASE_URL']!, '-X', '-v', 'ON_ERROR_STOP=1'], {
    input: `WITH content AS (SELECT convert_to(E'FAIL-FIRST\\n'||repeat(E'measurement failed\\n',10000)||E'FAIL-LAST\\n','UTF8') AS bytes) UPDATE diagnostic_artifact d SET raw_payload=c.bytes,export_payload=c.bytes,allocated_bytes=octet_length(c.bytes)*2,manifest=d.manifest||jsonb_build_object('purpose','report.md','media_type','text/markdown','original_bytes',octet_length(c.bytes),'retained_bytes',octet_length(c.bytes),'export_bytes',octet_length(c.bytes),'raw_sha256',encode(sha256(c.bytes),'hex'),'export_sha256',encode(sha256(c.bytes),'hex')) FROM content c WHERE d.artifact_id='${artifact}';`,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  await page.goto(`/requirements/${id}`);
  const panel = page.getByRole('region', { name: '失败诊断与报告' });
  await panel.getByText('report.md · 完整保留', { exact: false }).click();
  await panel.getByRole('button', { name: '查看脱敏内容' }).click();
  const content = page.getByRole('region', { name: '脱敏诊断内容' });
  await expect(content.locator('pre')).toContainText('FAIL-FIRST');
  await content.getByRole('button', { name: '下一段' }).click();
  await expect(content).toContainText('字节 8192');
  const manifestResponse = await page.request.get(`/api/requirements/${id}/diagnostics/0`);
  expect(manifestResponse.status()).toBe(200);
  const manifest = (await manifestResponse.json()).artifacts[0];
  const downloaded = page.waitForEvent('download');
  await panel.getByRole('button', { name: '下载已保留内容' }).click();
  const file = await downloaded;
  const bytes = readFileSync((await file.path())!);
  const { createHash } = await import('node:crypto');
  expect(bytes.byteLength).toBe(manifest.export_bytes);
  expect(createHash('sha256').update(bytes).digest('hex')).toBe(manifest.export_sha256);
  expect(bytes.toString()).toContain('FAIL-FIRST');
  expect(bytes.toString()).toContain('FAIL-LAST');
  await info.attach('diagnostic-download-identity.json', {
    body: JSON.stringify(
      {
        source: 'disposable persisted browser fixture',
        viewport: info.project.name,
        artifact: manifest,
        downloaded_bytes: bytes.byteLength,
        downloaded_sha256: createHash('sha256').update(bytes).digest('hex'),
      },
      null,
      2,
    ),
    contentType: 'application/json',
  });
  expect(
    (await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze())
      .violations,
  ).toEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath('diagnostics.png'), fullPage: true });
  execFileSync('psql', [process.env['TEST_DATABASE_URL']!, '-X', '-v', 'ON_ERROR_STOP=1'], {
    input: `UPDATE diagnostic_artifact SET manifest=manifest||'{"availability":"missing","reason":"storage quota exhausted; producer evidence retained"}'::jsonb,raw_payload=NULL,export_payload=NULL WHERE artifact_id='${artifact}';`,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  await panel.getByRole('button', { name: '刷新诊断清单' }).click();
  await expect(panel).toContainText('内容缺失');
  await expect(panel).toContainText('storage quota exhausted');
  await expect(panel.getByRole('button', { name: '下载已保留内容' })).toHaveCount(0);
});
