import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { RecoveryPanel } from './recovery-panel';
import { OperationDetail } from './operations';
import { GetExtensionRecoveryResponse } from './health-response';

const failure: GetExtensionRecoveryResponse['failures'][number] = {
  event_key: 'failure',
  validation_id: 'validation',
  decision: 'blocked',
  facts: {
    candidate_sha: 'a'.repeat(40),
    phase: 'local',
    feedback: {
      verdict: 'unknown',
      fault: {
        class: 'unsupported',
        owner: 'plugin_maintainer',
        resume_condition: 'review collector',
        scope: ['quality'],
      },
    },
  },
};
const detail = {
  requirement: { id: 1, version: 4, revision: 2, paused: false, cancel_requested: false },
  validations: [{ id: 'validation', revision: 2 }],
} as OperationDetail;

describe('Version-bound recovery decisions', () => {
  let http: HttpTestingController;
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    http = TestBed.inject(HttpTestingController);
    const fixture = TestBed.createComponent(RecoveryPanel);
    fixture.componentRef.setInput('item', detail);
    TestBed.tick();
    http.expectOne('/api/requirements/1/extension-recovery').flush({ failures: [failure] });
    return fixture;
  }
  afterEach(() => http.verify());
  it('submits exact version and candidate, prevents concurrent writes, and retains input on conflict', async () => {
    const fixture = setup();
    await fixture.whenStable();
    const page = fixture.componentInstance;
    expect(page.eligible(failure)).toBe(true);
    for (const current of [
      { ...failure, decision: 'code' },
      { ...failure, successor_validation: 'next' },
      { ...failure, resolution: {}, resolution_state: 'pending' },
    ]) {
      page.failures.set([current]);
      expect(page.eligible(failure)).toBe(false);
    }
    page.failures.set([failure]);
    expect(page.eligible({ ...failure, validation_id: 'other' })).toBe(false);
    page.choose(failure);
    page.model.update((m) => ({
      ...m,
      reason: 'approved implementation available',
      plan: 'a'.repeat(64),
      condition: 'review passed',
    }));
    const pending = page.submit();
    const req = http.expectOne('/api/requirements/1/extension-recovery');
    expect(req.request.body.version).toBe(4);
    expect(req.request.body.revision).toBe(2);
    expect(req.request.body.validation_id).toBe('validation');
    expect(req.request.body.action.kind).toBe('revalidate');
    await page.submit();
    req.flush({ error: 'stale' }, { status: 409, statusText: 'Conflict' });
    await pending;
    expect(page.model().reason).toBe('approved implementation available');
    const retry = page.submit();
    const replay = http.expectOne('/api/requirements/1/extension-recovery');
    expect(replay.request.body.request_id).toBe(req.request.body.request_id);
    replay.flush({
      accepted: true,
      started: false,
      version: 5,
      event_key: 'failure',
      resolution_state: 'pending',
    });
    await retry;
    expect(page.message()).toContain('尚未报告会话启动');
    expect(page.selected()).toBeNull();
    await page.submit();
  });
  it('validates the three recovery routes without broadening the approved code scope', async () => {
    const fixture = setup();
    await fixture.whenStable();
    const page = fixture.componentInstance;
    expect(() => page.action()).toThrow('恢复理由');
    page.model.update((m) => ({ ...m, reason: 'reviewed', kind: 'adapt_code' }));
    expect(() => page.action()).toThrow('代码约束');
    page.model.update((m) => ({
      ...m,
      instruction: 'Use named functions',
      paths: ' src/main.rs \n\ntests/flow.rs',
      release: 'reviewed collector',
    }));
    expect(page.action().constraints?.[0].paths).toEqual(['src/main.rs', 'tests/flow.rs']);
    page.model.update((m) => ({ ...m, kind: 'revalidate_delivery' }));
    expect(() => page.action()).toThrow('计划摘要');
    page.model.update((m) => ({ ...m, plan: 'b'.repeat(64), condition: 'installed' }));
    expect(() => page.action()).toThrow('交付策略');
    page.model.update((m) => ({ ...m, policy: 'c'.repeat(64) }));
    expect(page.action().kind).toBe('revalidate_delivery');
    page.choose(failure);
    const submit = page.submit();
    http
      .expectOne('/api/requirements/1/extension-recovery')
      .flush({ accepted: true, started: true });
    await submit;
    expect(page.message()).toContain('已报告启动');
    fixture.componentRef.setInput('disabled', true);
    expect(page.eligible(failure)).toBe(false);
  });
  it('ignores older reads, keeps drafts, and stops recovery while paused or facts are unavailable', async () => {
    const fixture = setup();
    await fixture.whenStable();
    const page = fixture.componentInstance;
    const old = page.load(1);
    const first = http.expectOne('/api/requirements/1/extension-recovery');
    const fresh = page.load(1);
    const last = http.expectOne('/api/requirements/1/extension-recovery');
    last.flush({ failures: [failure] });
    await fresh;
    first.flush({ failures: [] });
    await old;
    expect(page.failures()).toEqual([failure]);
    page.choose(failure);
    page.failures.set([]);
    expect(page.eligible(failure)).toBe(false);
    page.failures.set([{ ...failure, validation_id: 'new-validation' }]);
    expect(page.eligible(failure)).toBe(false);
    page.failures.set([
      { ...failure, resolution: { actor: 'operator' }, resolution_state: 'complete' },
    ]);
    expect(page.eligible(failure)).toBe(false);
    page.failures.set([failure]);
    const offline = page.load(1);
    http.expectOne('/api/requirements/1/extension-recovery').error(new ProgressEvent('error'));
    await offline;
    expect(page.eligible(failure)).toBe(false);
    fixture.componentRef.setInput('item', {
      ...detail,
      requirement: { ...detail.requirement, paused: true },
    });
    TestBed.tick();
    http.expectOne('/api/requirements/1/extension-recovery').flush({ failures: [failure] });
    await fixture.whenStable();
    expect(page.eligible(failure)).toBe(false);
    page.choose(failure);
    await page.submit();
  });
});
