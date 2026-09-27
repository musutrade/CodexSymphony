import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { LifecyclePanel } from './lifecycle-panel';
import { GetLifecycleHistoryResponse } from './health-response';

const event: GetLifecycleHistoryResponse['events'][number] = {
  event_id: 12,
  sequence: 100,
  phase: 'validation',
  deliveries: [{ plugin_id: 'bark', state: 'failed', attempts: 3 }],
};
describe('Durable lifecycle and actual model facts', () => {
  let http: HttpTestingController;
  function flush(events = [event]) {
    http.expectOne('/api/requirements/1/lifecycle?after=0').flush({ events });
    http.expectOne('/api/requirements/1/models').flush({
      runs: [{ run_id: 'run', revision: 1, actual: null, frozen: null, matched: null, usage: [] }],
    });
  }
  function setup() {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
    http = TestBed.inject(HttpTestingController);
    const fixture = TestBed.createComponent(LifecyclePanel);
    fixture.componentRef.setInput('requirementId', 1);
    fixture.componentRef.setInput('version', 1);
    TestBed.tick();
    flush();
    return fixture;
  }
  afterEach(() => http.verify());
  it('paginates bounded source events and replays only the selected plugin without rerunning business', async () => {
    const fixture = setup();
    await fixture.whenStable();
    const page = fixture.componentInstance;
    const refresh = page.refresh();
    flush(Array.from({ length: 100 }, (_, i) => ({ ...event, event_id: i + 1, sequence: i + 1 })));
    await refresh;
    expect(page.more()).toBe(true);
    const next = page.next();
    http
      .expectOne('/api/requirements/1/lifecycle?after=100')
      .flush({ events: [{ ...event, event_id: 101, sequence: 101 }] });
    await next;
    expect(page.events()).toHaveLength(101);
    expect(page.more()).toBe(false);
    const replay = page.replay(event, 'bark');
    const req = http.expectOne('/api/requirements/1/notifications/replay');
    expect(req.request.body).toEqual({ event_id: 12, plugin_id: 'bark' });
    await page.replay(event, 'bark');
    req.flush({ accepted: true, started: false });
    await Promise.resolve();
    flush();
    await replay;
    expect(page.message()).toContain('尚未报告渠道发送');
    const refused = page.replay(event, 'bark');
    http
      .expectOne('/api/requirements/1/notifications/replay')
      .flush({ accepted: false, started: false });
    await Promise.resolve();
    flush();
    await refused;
    expect(page.message()).toContain('未接受');
  });
  it('does not use stale reads or failures as current notification authority', async () => {
    const fixture = setup();
    await fixture.whenStable();
    const page = fixture.componentInstance;
    const old = page.refresh();
    const oldEvents = http.expectOne('/api/requirements/1/lifecycle?after=0');
    const oldModels = http.expectOne('/api/requirements/1/models');
    const fresh = page.refresh();
    flush([]);
    await fresh;
    oldEvents.flush({ events: [event] });
    oldModels.flush({ runs: [] });
    await old;
    expect(page.events()).toEqual([]);
    const failed = page.next();
    http.expectOne('/api/requirements/1/lifecycle?after=0').error(new ProgressEvent('error'));
    await failed;
    await page.next();
    await page.replay(event, 'bark');
    expect(page.error()).toContain('暂不可读');
    const offline = page.refresh();
    http.expectOne('/api/requirements/1/lifecycle?after=0').error(new ProgressEvent('error'));
    http.expectOne('/api/requirements/1/models').flush({ runs: [] });
    await offline;
    expect(page.error()).toContain('陈旧');
    const recovered = page.refresh();
    flush();
    await recovered;
    const unknown = page.replay(event, 'bark');
    http.expectOne('/api/requirements/1/notifications/replay').error(new ProgressEvent('error'));
    await Promise.resolve();
    flush();
    await unknown;
    expect(page.message()).toContain('不会自动重放');
    fixture.componentRef.setInput('disabled', true);
    await page.replay(event, 'bark');
  });
});
