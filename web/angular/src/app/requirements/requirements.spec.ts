import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { Requirements } from './requirements';
import { vi } from 'vitest';

const contract = {
  title: 'Future test',
  description: 'Description',
  acceptance_criteria: [{ description: 'Passes', verification_ref: 'test' }],
  validation_plan: [
    {
      id: 'test',
      check: 'cargo_test',
      selector: 'future::test',
      expected_result: 'exit 0',
      timeout_seconds: 60,
    },
  ],
  network_access: [],
};
const repository = {
  project: 'Project',
  remote: 'owner/repo',
  github_repository_id: 1,
  base_branch: 'main',
  revoked: false,
  reason: 'review',
  policy: {
    allowed_checks: ['cargo_test'],
    max_timeout_seconds: 120,
    token_limit: 1000,
    turn_limit: 10,
    model_work_seconds: 600,
    gate_recovery_policy: 'one_code_repair',
  },
};
const context = {
  repositories: [{ version: 1, repository }],
  deployment_network: [],
  network_status: 'not_configured',
  runtime_ready: false,
  repository_ready: false,
};
const item = {
  id: 1,
  version: 1,
  state: 'Draft',
  revision: 0,
  creator: 'local-user',
  created_at: 'now',
  contract,
  snapshots: [],
  authorization_valid: false,
};

describe('Requirement browser workflow', () => {
  let http: HttpTestingController;
  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [Requirements],
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    http = TestBed.inject(HttpTestingController);
  });
  afterEach(() => http.verify());
  async function start() {
    const fixture = TestBed.createComponent(Requirements);
    TestBed.tick();
    http.expectOne('/api/multi/repository').flush(context);
    http.expectOne('/api/multi/requirements').flush({ requirements: [] });
    await vi.waitFor(() => expect(fixture.componentInstance.loading()).toBe(false));
    fixture.detectChanges();
    return fixture;
  }
  it('loads, validates, registers and reports a failed request without losing input', async () => {
    const fixture = await start();
    const c = fixture.componentInstance;
    expect(fixture.nativeElement.textContent).toContain('暂无需求');
    await c.configure();
    expect(c.setup.touched).toBe(true);
    c.setup.patchValue({ project: 'Project', remote: 'owner/repo' });
    const configured = c.configure();
    const req = http.expectOne('/api/multi/repository');
    expect(req.request.method).toBe('PUT');
    expect(req.request.headers.get('x-codexsymphony-csrf')).toBe('1');
    req.flush({ version: 1, repository });
    await Promise.resolve();
    http.expectOne('/api/multi/repository').flush(context);
    await configured;
    await c.save();
    expect(c.form.touched).toBe(true);
    c.form.patchValue({ ...contract, network_access: '' });
    const failed = c.save();
    const first = http.expectOne('/api/multi/requirements');
    const key = first.request.body.request_id;
    first.flush({}, { status: 503, statusText: 'Unavailable' });
    await failed;
    expect(c.error()).toContain('操作未完成');
    expect(c.form.controls.title.value).toBe('Future test');
    const retry = c.save();
    const second = http.expectOne('/api/multi/requirements');
    expect(second.request.body.request_id).toBe(key);
    second.flush(item);
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await retry;
    expect(c.selected()?.id).toBe(1);
  });
  it('edits, reviews the observed policy version, withdraws and preserves snapshots', async () => {
    const fixture = await start();
    const c = fixture.componentInstance;
    const opened = c.open(1);
    http.expectOne('/api/multi/requirements/1').flush(item);
    await opened;
    c.addCriterion();
    c.form.controls.acceptance_criteria.removeAt(1);
    c.addStep();
    c.form.controls.validation_plan.removeAt(1);
    const saved = c.save();
    const edit = http.expectOne('/api/multi/requirements/1');
    expect(edit.request.method).toBe('PATCH');
    edit.flush({ ...item, version: 2 });
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await saved;
    const reviewed = c.control('ready');
    const ready = http.expectOne('/api/multi/requirements/1/ready');
    expect(ready.request.body.repository_version).toBe(1);
    ready.flush({
      ...item,
      version: 3,
      state: 'Ready',
      revision: 1,
      snapshots: [
        {
          revision: 1,
          contract,
          repository,
          repository_version: 1,
          reviewer: 'local-user',
          ac_ids: ['AC-1-1-1'],
        },
      ],
    });
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await reviewed;
    expect(c.message()).toContain('Ready 已持久化');
    const withdrawn = c.control('withdraw');
    http.expectOne('/api/multi/requirements/1/withdraw').flush({ ...item, version: 4 });
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await withdrawn;
    expect(c.message()).toContain('已撤回');
    c.newDraft();
    expect(c.selected()).toBeUndefined();
    await c.control('ready');
    expect(() => c.accept({ error: 'not found' })).toThrow();
  });
  it('renders a load failure then recovers with a fresh read', async () => {
    const fixture = TestBed.createComponent(Requirements);
    TestBed.tick();
    http.expectOne('/api/multi/repository').flush({}, { status: 503, statusText: 'Unavailable' });
    http.expectOne('/api/multi/requirements').flush({ requirements: [] });
    await fixture.whenStable();
    await vi.waitFor(() => expect(fixture.componentInstance.error()).toContain('操作未完成'));
    const reloaded = fixture.componentInstance.reload();
    http.expectOne('/api/multi/repository').flush(context);
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await reloaded;
    expect(fixture.componentInstance.items().length).toBe(1);
  });
  it('saves explicit repository selection and reviews its own policy version', async () => {
    const fixture = await start();
    const c = fixture.componentInstance;
    c.repository.set({
      ...context,
      repositories: [
        { id: 1, version: 1, repository },
        { id: 2, version: 7, repository: { ...repository, remote: 'owner/second' } },
      ],
    });
    c.accept({ ...item, repository_id: 2 });
    c.selectRepository(1);
    expect(c.form.dirty).toBe(true);
    c.selectRepository(2);
    const saving = c.save();
    const request = http.expectOne('/api/multi/requirements/1');
    expect(request.request.body.repository_id).toBe(2);
    request.flush({ ...item, repository_id: 2 });
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await saving;
    const reviewing = c.control('ready');
    const review = http.expectOne('/api/multi/requirements/1/ready');
    expect(review.request.body.repository_version).toBe(7);
    review.flush({ ...item, repository_id: 2, state: 'Ready' });
    await Promise.resolve();
    http.expectOne('/api/multi/requirements').flush({ requirements: [item] });
    await reviewing;
  });
});
