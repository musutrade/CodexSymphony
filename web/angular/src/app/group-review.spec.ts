import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { RouterTestingHarness } from '@angular/router/testing';
import { routes } from './app.routes';
import { ActivatedRoute, convertToParamMap, provideRouter } from '@angular/router';
import { GroupReview } from './group-review';
import { GroupView, initialReview, editableReview, totalBudget } from './group-review-model';
import { draftExample } from './drafts-example';
const fixtureView: GroupView = {
  draft_id: 'draft-group',
  draft_revision: 1,
  version: 0,
  document: JSON.parse(draftExample) as GroupView['document'],
  review: null,
  repositories: [
    {
      id: 1,
      version: 1,
      repository: {
        project: 'fixture',
        remote: 'test/group',
        github_repository_id: 12,
        base_branch: 'main',
        revoked: false,
        reason: 'fixture',
        policy: {
          allowed_checks: ['cargo_test'],
          max_timeout_seconds: 120,
          token_limit: 100,
          turn_limit: 2,
          model_work_seconds: 60,
          gate_recovery_policy: 'one_code_repair',
        },
      },
    },
  ],
  budgets: [],
  queue: null,
  authorizations: [],
  scheduler_available: false,
  business_complete: false,
};
describe('Atomic group review UI', () => {
  function setup() {
    TestBed.configureTestingModule({
      imports: [GroupReview],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        provideRouter([]),
        {
          provide: ActivatedRoute,
          useValue: { snapshot: { paramMap: convertToParamMap({ id: 'draft-group' }) } },
        },
      ],
    });
    const fixture = TestBed.createComponent(GroupReview);
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/drafts/draft-group/review').flush(fixtureView);
    return { fixture, http, page: fixture.componentInstance };
  }
  it('shows owner, dependencies, progress and explicit unsupported execution', async () => {
    const { fixture, page, http } = setup();
    await fixture.whenStable();
    page.view.set({
      ...fixtureView,
      execution: {
        owner: 19,
        paused: true,
        completed: 1,
        total: 2,
        parent_state: 'waiting_business_acceptance',
        items: [
          {
            child_id: 'C1',
            kind: 'code_change',
            order: 1,
            depends_on: [],
            repository_id: 1,
            requirement_id: 19,
            state: 'Submitted',
            owner: true,
            complete: false,
            waiting_reason: 'waiting_confirmed_merge_and_applicable_acceptance',
          },
          {
            child_id: 'C2',
            kind: 'validation_only',
            order: 2,
            depends_on: ['C1'],
            repository_id: 2,
            requirement_id: null,
            state: 'Queued',
            owner: false,
            complete: false,
            waiting_reason: 'waiting_validation_only_execution_not_implemented',
          },
        ],
      },
    });
    await fixture.whenStable();
    const text = fixture.nativeElement.textContent as string;
    expect(text).toContain('全局占用者：19');
    expect(text).toContain('当前执行占用者');
    expect(text).toContain('父项等待整体验收');
    expect(text).toContain('不会创建编码 Run 或空 PR');
    expect(page.waitingReason('unknown')).toBe('unknown');
    http.verify();
  });
  it('reviews persisted revisions, maps coverage, edits budgets, saves and confirms once', async () => {
    const { fixture, http, page } = setup();
    await fixture.whenStable();
    expect(page.total().tokens).toBe(200);
    page.fullChain('AC01', { target: { checked: false } } as unknown as Event);
    page.fullChain('AC01', { target: { checked: true } } as unknown as Event);
    page.addMapping('AC01');
    page.addMapping('AC01');
    page.removeMapping(0);
    expect(page.model().coverage.length).toBe(1);
    expect(page.clean()).toBe(false);
    page.model.update((m) => ({
      ...m,
      semantic_review: 'matches parent flow',
      automatic_budget: false,
      group_budget: { tokens: 150, turns: 3, model_seconds: 90 },
    }));
    expect(page.total().tokens).toBe(150);
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('不能证明实现或业务验收完成');
    const saving = page.save();
    const write = http.expectOne('/api/drafts/draft-group/review');
    expect(write.request.method).toBe('PUT');
    expect(write.request.body.review.group_budget.tokens).toBe(150);
    const saved = { ...fixtureView, version: 1, review: page.payload() };
    write.flush(saved);
    await saving;
    expect(page.clean()).toBe(true);
    const confirming = page.confirm();
    const confirm = http.expectOne('/api/drafts/draft-group/authorize');
    expect(confirm.request.body.version).toBe(1);
    expect(confirm.request.headers.get('x-codexsymphony-csrf')).toBe('1');
    await page.confirm();
    http.expectNone('/api/drafts/draft-group/authorize');
    confirm.flush({
      authorization_id: 7,
      state: 'waiting_scheduler',
      scheduler_available: false,
      business_complete: false,
    });
    await Promise.resolve();
    http
      .expectOne('/api/drafts/draft-group/review')
      .flush({ ...saved, queue: { version: 1, authorization_id: 7, state: 'waiting_scheduler' } });
    await confirming;
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('编辑未开始队列');
    expect(page.message()).toContain('依赖队列');
    page.resetForRevision();
    expect(page.model().semantic_review).toBe('');
    http.verify();
  });
  it('retains edits and retry identity on errors, reloads historical usage', async () => {
    const { fixture, http, page } = setup();
    await fixture.whenStable();
    page.model.update((m) => ({ ...m, semantic_review: 'retain this' }));
    const fail = page.save();
    http
      .expectOne('/api/drafts/draft-group/review')
      .flush({ error: 'version conflict' }, { status: 409, statusText: 'Conflict' });
    await fail;
    expect(page.error()).toContain('version conflict');
    expect(page.model().semantic_review).toBe('retain this');
    const confirm = page.confirm();
    const request = http.expectOne('/api/drafts/draft-group/authorize');
    const key = request.request.body.request_id;
    request.flush({ error: 'budget insufficient' });
    await confirm;
    expect(page.error()).toContain('budget insufficient');
    const retry = page.confirm();
    const second = http.expectOne('/api/drafts/draft-group/authorize');
    expect(second.request.body.request_id).toBe(key);
    second.flush('offline', { status: 503, statusText: 'Unavailable' });
    await retry;
    expect(page.error()).toContain('offline');
    const reload = page.reload();
    http.expectOne('/api/drafts/draft-group/review').flush({
      ...fixtureView,
      budgets: [
        {
          item_id: '',
          limits: { tokens: 100, turns: 2, model_seconds: 60 },
          used: { tokens: 10, turns: 1, model_seconds: 2 },
          reserved: { tokens: 5, turns: 1, model_seconds: 3 },
        },
      ],
    });
    await reload;
    expect(page.budget('').used.tokens).toBe(10);
    expect(page.budget('missing').used.tokens).toBe(0);
    const invalid = page.reload();
    http.expectOne('/api/drafts/draft-group/review').flush({ error: 'draft missing' });
    await invalid;
    expect(page.error()).toContain('draft missing');
    const rejected = page.save();
    http.expectOne('/api/drafts/draft-group/review').flush({ error: 'rejected' });
    await rejected;
    expect(page.error()).toContain('rejected');
    http.verify();
  });
  it('loads the group review route by stable Draft identity', async () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter(routes)],
    });
    const http = TestBed.inject(HttpTestingController);
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/drafts/draft-group/review', GroupReview);
    http.expectOne('/api/drafts/draft-group/review').flush(fixtureView);
    await harness.fixture.whenStable();
    expect(harness.routeNativeElement?.textContent).toContain('整组评审与授权');
    http.verify();
  });
  it('builds an incomplete form for missing repository without fabricating authority', () => {
    const review = initialReview({ ...fixtureView, repositories: [] });
    expect(review.items[0].repository_version).toBe(0);
    expect(totalBudget(review.items).tokens).toBe(0);
    expect(editableReview(review).automatic_budget).toBe(true);
  });
});
