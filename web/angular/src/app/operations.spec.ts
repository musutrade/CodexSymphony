import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router';
import { of } from 'rxjs';
import { vi } from 'vitest';
import { OperationDetail, Operations } from './operations';

const item: OperationDetail = {
  requirement: {
    id: 1,
    version: 1,
    revision: 1,
    state: 'Running',
    paused: true,
    cancel_requested: false,
    cleanup_complete: false,
  },
  runs: [],
  validations: [],
  external: [],
  events: [],
  preparation: [],
  storage: { blocked: false, error: null },
  storage_lifecycle: 'not_ready',
  materials: [
    { run_id: 'run', channel: 'stdout', kept_bytes: 12, discarded_bytes: 0, status: 'available' },
  ],
  questions: [
    {
      id: 'question',
      version: 1,
      revision: 1,
      run_id: 'run',
      questions: [{ id: 'choice', question: 'Which option?', options: [] }],
      answered: false,
      resume_state: 'waiting',
    },
  ],
  metrics: {
    input: null,
    cached: null,
    output: null,
    model_calls: 0,
    repair_count: 0,
    human_seconds: null,
    interventions: 0,
    reasons: [],
    phases: [],
    to_pr_seconds: null,
    zero_intervention: { phase: 'reviewed_to_submitted', numerator: 0, denominator: 0 },
  },
};

describe('Durable operator UI', () => {
  let http: HttpTestingController;
  function setup(inbox = false) {
    const params = convertToParamMap(inbox ? {} : { id: '1' });
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: { snapshot: { paramMap: params }, paramMap: of(params) },
        },
      ],
      imports: [Operations],
    });
    http = TestBed.inject(HttpTestingController);
    return TestBed.createComponent(Operations);
  }
  afterEach(() => http.verify());
  async function read(value = item) {
    await vi.waitFor(() => http.expectOne('/api/requirements/1/operations').flush(value));
    await Promise.resolve();
    await Promise.resolve();
  }
  it('shows retained identities and capacity without claiming replay, and authorizes a bounded recheck', async () => {
    const fixture = setup();
    const stored: OperationDetail = {
      ...item,
      storage_usage: {
        configured: true,
        policy_version: 'storage-v1',
        actual_bytes: 1024,
        available_bytes: 8192,
        measured_at: 200,
        control_bytes: 1024,
        global_limit: 16384,
        reserved_bytes: 4096,
        requirement_allocated: 6000,
        protected_bytes: 512,
        cleanup_todo: 1,
        classification_todo: 2,
        categories: [
          {
            name: 'cold',
            actual_bytes: 512,
            reserved_bytes: 1024,
            limit_bytes: 8192,
            retention_seconds: 60,
          },
        ],
        materials: [
          {
            id: 'material',
            run_id: 'old-run',
            kind: 'retrospective',
            category: 'hot',
            status: 'deleted',
            actual_bytes: 0,
            expires_at: 100,
            protection: null,
            attempts: 2,
            next_attempt_at: null,
            failure: null,
            deleted_at: 200,
            reason: 'expired',
            replacement: 'verified-record',
            resolved_by: 'new-run',
            identity: 'exact-candidate-sha',
            summary: 'retained failure reason',
          },
        ],
      },
    };
    await read(stored);
    await fixture.whenStable();
    const text = fixture.nativeElement.textContent;
    expect(text).toContain('6000');
    expect(text).toContain('exact-candidate-sha');
    expect(text).toContain('verified-record');
    const pending = fixture.componentInstance.control(stored, 'storage_recheck');
    const request = http.expectOne('/api/requirements/1/operations');
    expect(request.request.body.action).toBe('storage_recheck');
    request.flush({ version: 2 });
    await read(stored);
    await pending;
  });
  it('refreshes conflicts, suppresses concurrent submissions and retains retry identity', async () => {
    const fixture = setup();
    await read();
    await fixture.whenStable();
    const component = fixture.componentInstance;
    const pending = component.control(item, 'resume');
    const request = http.expectOne('/api/requirements/1/operations');
    const key = request.request.body.request_id;
    await component.control(item, 'resume');
    http.expectNone('/api/requirements/1/operations');
    request.flush({}, { status: 409, statusText: 'Conflict' });
    await read();
    await pending;
    expect(component.message()).toContain('状态已变化');
    const retry = component.control(item, 'resume');
    const retried = http.expectOne('/api/requirements/1/operations');
    expect(retried.request.body.request_id).toBe(key);
    retried.flush({ version: 2 });
    await read({ ...item, requirement: { ...item.requirement, version: 2, paused: false } });
    await retry;
    expect(component.items()[0].requirement.paused).toBe(false);
  });
  it('does not allow an older polling result to overwrite a post-action refresh', async () => {
    const fixture = setup();
    const old = http.expectOne('/api/requirements/1/operations');
    const refresh = fixture.componentInstance.reload();
    await read({ ...item, requirement: { ...item.requirement, version: 3 } });
    await refresh;
    old.flush(item);
    await fixture.whenStable();
    expect(fixture.componentInstance.items()[0].requirement.version).toBe(3);
  });
  it('keeps unsent answers bound to the question version after polling', async () => {
    const fixture = setup();
    await read();
    const component = fixture.componentInstance;
    const question = item.questions[0];
    component.answerControl(question, 'choice').setValue('old scope');
    const changed = { ...question, version: 2 };
    const loading = component.reload();
    await read({ ...item, questions: [changed] });
    await loading;
    expect(component.answerControl(changed, 'choice').value).toBe('');
    expect(component.message()).toContain('问题已更新');
    expect(component.answerControl(question, 'choice').value).toBe('old scope');
    await component.answer(changed);
    http.expectNone('/api/operator/questions/question/answer');
  });
  it('associates required answer errors and saves answers independently of pause', async () => {
    const fixture = setup();
    await read();
    const component = fixture.componentInstance;
    const question = item.questions[0];
    await fixture.whenStable();
    await component.answer(question);
    TestBed.tick();
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('input').getAttribute('aria-describedby')).toMatch(
      /mat-mdc-error/,
    );
    const input = fixture.nativeElement.querySelector('input') as HTMLInputElement;
    input.value = 'yes';
    input.dispatchEvent(new Event('input'));
    const answer = component.answer(question);
    const request = http.expectOne('/api/operator/questions/question/answer');
    expect(request.request.body).toEqual({ version: 1, answers: [{ id: 'choice', text: 'yes' }] });
    expect(request.request.headers.get('x-codexsymphony-csrf')).toBe(null);
    request.flush({ saved: true });
    await read({ ...item, questions: [{ ...question, answered: true, resume_state: 'pending' }] });
    await answer;
    expect(component.items()[0].requirement.paused).toBe(true);
    const preview = component.evidence(1, item.materials[0]);
    http
      .expectOne('/api/requirements/1/evidence/run/stdout')
      .flush({ text: 'safe', preview_only: true });
    await read();
    await preview;
    expect(component.preview()).toBe('safe');
    expect(component.prLink('owner/repo', 42)).toBe('https://github.com/owner/repo/pull/42');
    expect(component.prLink('javascript:alert(1)', 42)).toBeNull();
    expect(component.prLink('owner/repo', null)).toBeNull();
  });
  it('polls only while idle and stops all reads when the page closes', async () => {
    vi.useFakeTimers();
    try {
      const fixture = setup();
      http.expectOne('/api/requirements/1/operations').flush(item);
      await vi.advanceTimersByTimeAsync(0);
      await vi.advanceTimersByTimeAsync(5000);
      http.expectOne('/api/requirements/1/operations').flush(item);
      await vi.advanceTimersByTimeAsync(0);
      fixture.componentInstance.busy.set(true);
      await vi.advanceTimersByTimeAsync(5000);
      http.expectNone('/api/requirements/1/operations');
      fixture.destroy();
      await vi.advanceTimersByTimeAsync(5000);
      http.expectNone('/api/requirements/1/operations');
    } finally {
      vi.useRealTimers();
    }
  });
  it('handles empty inbox, network failure and recovery without lifecycle writes', async () => {
    const fixture = setup(true);
    http.expectOne('/api/inbox').flush({ requirement_ids: [] });
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('暂无待办');
    const failure = fixture.componentInstance.reload();
    http.expectOne('/api/inbox').flush({}, { status: 503, statusText: 'Unavailable' });
    await failure;
    expect(fixture.componentInstance.error()).toContain('陈旧');
    const recovery = fixture.componentInstance.reload();
    http.expectOne('/api/inbox').flush({ requirement_ids: [1] });
    await read();
    await recovery;
    expect(fixture.componentInstance.error()).toBe('');
  });
});
