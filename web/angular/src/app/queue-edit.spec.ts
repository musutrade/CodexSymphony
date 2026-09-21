import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { signal } from '@angular/core';
import { QUEUE_EDIT_CONTEXT } from './queue-edit-context';
import { TestBed } from '@angular/core/testing';
import { QueueEdit } from './queue-edit';
import { GroupView, initialReview } from './group-review-model';
import { draftExample } from './drafts-example';
function view(): GroupView {
  const result: GroupView = {
    draft_id: 'draft-edit',
    draft_revision: 1,
    version: 1,
    document: JSON.parse(draftExample) as GroupView['document'],
    review: null,
    repositories: [],
    budgets: [],
    queue: { version: 1, authorization_id: 1, state: 'waiting_scheduler' },
    authorizations: [],
    scheduler_available: true,
    business_complete: false,
  };
  result.review = initialReview(result);
  result.review.coverage = [
    { parent_ac: 'P1', child_id: 'C1', child_revision: 1, child_ac: 'AC1', step_id: 'test' },
  ];
  return result;
}
describe('Queue delta review', () => {
  function setup() {
    const currentView = signal<GroupView | undefined>(view());
    TestBed.configureTestingModule({
      imports: [QueueEdit],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        { provide: QUEUE_EDIT_CONTEXT, useValue: { view: currentView, reload: () => Promise.resolve() } },
      ],
    });
    const fixture = TestBed.createComponent(QueueEdit);
    fixture.detectChanges();
    return {
      currentView,
      fixture,
      component: fixture.componentInstance,
      http: TestBed.inject(HttpTestingController),
    };
  }
  it('prefills exact next revisions, preserves invalid input and submits a proposal', async () => {
    const { component, http } = setup();
    component.begin();
    expect(JSON.parse(component.model().review).parent_revision).toBe(2);
    const saved = component.model();
    component.model.set({ ...saved, document: 'broken' });
    await component.propose();
    expect(component.error()).toContain('JSON');
    component.model.set(saved);
    const action = component.propose();
    const req = http.expectOne('/api/drafts/draft-edit/queue-edit');
    expect(req.request.body.change.kind).toBe('propose');
    expect(req.request.headers.get('x-codexsymphony-csrf')).toBe(null);
    req.flush({ version: 2, affected: ['C1'] });
    await action;
    expect(component.message()).toContain('C1');
    expect(component.editing()).toBe(false);
    http.verify();
  });
  it('reorders with CAS and retains conflict details and local input', async () => {
    const { component, http } = setup();
    const original = component.ordered().map((c) => c.id);
    const action = component.move(0, 1);
    const req = http.expectOne('/api/drafts/draft-edit/queue-edit');
    expect(req.request.body.version).toBe(1);
    expect(req.request.body.change.order[0]).toBe(original[1]);
    await component.move(0, 1);
    req.flush({ error: 'queue version conflict' }, { status: 409, statusText: 'Conflict' });
    await action;
    expect(component.error()).toContain('queue version conflict');
    expect(component.error()).toContain('重新读取');
    http.verify();
  });
  it('approves the persisted edit version and handles explicit server errors', async () => {
    const { component, fixture, http, currentView } = setup();
    await component.approve();
    http.expectNone('/api/drafts/draft-edit/queue-edit');
    const current = view();
    current.pending_edit = {
      version: 2,
      document: current.document,
      review: current.review!,
      affected: ['C1'],
      repositories: [],
    };
    current.queue!.version = 2;
    currentView.set(current);
    fixture.detectChanges();
    component.begin();
    expect(component.model().document).toContain('schema');
    const rejected = component.approve();
    http.expectOne('/api/drafts/draft-edit/queue-edit').flush({ error: 'coverage missing' });
    await rejected;
    expect(component.error()).toContain('coverage missing');
    const accepted = component.approve();
    const req = http.expectOne('/api/drafts/draft-edit/queue-edit');
    expect(req.request.body.change.edit_version).toBe(2);
    req.flush({ version: 3, affected: [] });
    await accepted;
    expect(component.message()).toContain('无内容变化');
    http.verify();
  });
});
