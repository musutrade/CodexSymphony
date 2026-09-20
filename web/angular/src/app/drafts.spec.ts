import { vi } from 'vitest';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { Drafts } from './drafts';
import { draftExample } from './drafts-example';
import { routes } from './app.routes';
import { RouterTestingHarness } from '@angular/router/testing';
const draft = {
  id: 'draft-test',
  version: 1,
  state: 'Draft',
  document: JSON.parse(draftExample) as unknown,
  source: { format: 'json', label: 'fixture', text: draftExample },
  source_sha256: 'abc',
  warnings: [],
};
describe('Parent/child Draft entry', () => {
  function setup() {
    TestBed.configureTestingModule({
      imports: [Drafts],
      providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([])],
    });
    const fixture = TestBed.createComponent(Drafts);
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/drafts').flush({ drafts: [] });
    return { fixture, http, page: fixture.componentInstance };
  }
  it('imports, reads and edits versioned source while keeping execution unavailable', async () => {
    const { fixture, http, page } = setup();
    await fixture.whenStable();
    page.example();
    expect(page.model().text).toBe(draftExample);
    page.model.update((v) => ({ ...v, label: 'fixture' }));
    const save = page.save();
    const created = http.expectOne('/api/drafts');
    expect(created.request.method).toBe('POST');
    expect(created.request.body.version).toBe(0);
    expect(created.request.headers.get('x-codexsymphony-csrf')).toBe('1');
    created.flush(draft);
    await Promise.resolve();
    http
      .expectOne('/api/drafts')
      .flush({ drafts: [{ id: draft.id, version: 1, state: 'Draft', goal: 'fixture' }] });
    await save;
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('C2');
    expect(page.children()[1].depends_on).toEqual(['C1']);
    expect(page.message()).toContain('未授权执行');
    const edit = page.save();
    const request = http.expectOne('/api/drafts/draft-test');
    expect(request.request.method).toBe('PUT');
    expect(request.request.body.version).toBe(1);
    request.flush({ ...draft, version: 2 });
    await Promise.resolve();
    http.expectOne('/api/drafts').flush({ drafts: [] });
    await edit;
    const open = page.open('draft-test');
    http.expectOne('/api/drafts/draft-test').flush({ ...draft, version: 2 });
    await open;
    expect(page.model()).toEqual(draft.source);
    expect(page.message()).toContain('版本 2');
    page.newDraft();
    expect(page.selected()).toBeUndefined();
    expect(page.model().text).toBe('');
    expect(page.children()).toEqual([]);
    http.verify();
  });
  it('preserves input on conflict, validation and network errors, and blocks duplicate submission', async () => {
    const { fixture, http, page } = setup();
    await fixture.whenStable();
    page.example();
    const loading = page.open('draft-test');
    http.expectOne('/api/drafts/draft-test').flush(draft);
    await loading;
    for (const status of [409, 422, 503]) {
      const save = page.save();
      await page.save();
      http
        .expectOne('/api/drafts/draft-test')
        .flush(status === 503 ? {} : { error: 'invalid dependency' }, {
          status,
          statusText: 'rejected',
        });
      await save;
      expect(page.model().text).toBe(draftExample);
      expect(page.error().length).toBeGreaterThan(5);
      expect(page.busy()).toBe(false);
    }
    const read = page.open('draft-test');
    http.expectOne('/api/drafts/draft-test').flush({ error: 'draft not found' });
    await read;
    expect(page.error()).toContain('draft not found');
    const edit = page.save();
    http.expectOne('/api/drafts/draft-test').flush({ error: 'typed rejection' });
    await edit;
    expect(page.error()).toContain('typed rejection');
    const reload = page.reload();
    http.expectOne('/api/drafts').error(new ProgressEvent('network'));
    await reload;
    expect(page.error()).toContain('连接');
    http.verify();
  });
  it('loads the lazy import route and announces a failed list', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    });
    const harness = await RouterTestingHarness.create('/drafts');
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/drafts').flush({}, { status: 503, statusText: 'offline' });
    await harness.fixture.whenStable();
    harness.detectChanges();
    await vi.waitFor(() => {
      harness.detectChanges();
      expect(harness.routeNativeElement?.querySelector('[role=alert]')?.textContent).toContain(
        '请求失败',
      );
    });
    http.verify();
  });
});
