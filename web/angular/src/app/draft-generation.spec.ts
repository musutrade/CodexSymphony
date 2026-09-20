import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { DraftGeneration, GenerationRecord } from './draft-generation';
const record: GenerationRecord = {
  id: 'generation-1',
  draft_id: 'draft-generation-1',
  input_version: 0,
  output_version: 1,
  status: 'succeeded',
  error: null,
  request: {
    request_id: 'generation-1',
    draft_id: null,
    version: 0,
    label: 'source',
    text: 'input',
  },
  fingerprint: 'hash',
  output: '{}',
  evidence: {},
  usage: { input: 20, cached: 0, output: 10, model_seconds: 1, complete: true },
  limits: { tokens: 30000, turns: 1, model_seconds: 120 },
  created_at: '2026-09-20',
  completed_at: '2026-09-20',
};
function setup() {
  TestBed.configureTestingModule({ providers: [provideHttpClient(), provideHttpClientTesting()] });
  return { page: TestBed.inject(DraftGeneration), http: TestBed.inject(HttpTestingController) };
}
describe('advisory draft generation', () => {
  it('submits explicitly, reuses identity after lost response, preserves edits and loads persisted results', async () => {
    const { page, http } = setup();
    http.expectNone('/api/draft-generations');
    page.model.set({ label: 'source', text: 'input' });
    const pending = page.generate();
    await page.generate();
    const first = http.expectOne('/api/draft-generations');
    const id = first.request.body.request_id;
    expect(first.request.headers.get('x-codexsymphony-csrf')).toBe('1');
    first.error(new ProgressEvent('network'));
    await pending;
    expect(page.error()).toContain('请求 ID');
    expect(page.model().text).toBe('input');
    const retry = page.generate();
    const second = http.expectOne('/api/draft-generations');
    expect(second.request.body.request_id).toBe(id);
    second.flush(record);
    await retry;
    expect(page.records()).toEqual([record]);
    const reload = page.reload();
    http.expectOne('/api/draft-generations').flush({ generations: [record] });
    await reload;
    expect(page.stateLabel('running')).toBe('生成中');
    expect(page.stateLabel('failed')).toBe('生成失败');
    expect(page.stateLabel('interrupted')).toContain('中断');
    expect(page.stateLabel('conflict')).toContain('冲突');
    http.verify();
  });
  it('binds edits to selected version and presents typed failures and unknown usage', async () => {
    const { page, http } = setup();
    page.model.set({ label: 'edited', text: 'new input' });
    const pending = page.generate({ id: 'draft-old', version: 3 });
    const post = http.expectOne('/api/draft-generations');
    expect(post.request.body.version).toBe(3);
    expect(post.request.body.draft_id).toBe('draft-old');
    post.flush({ error: 'version conflict' }, { status: 409, statusText: 'Conflict' });
    await pending;
    expect(page.error()).toBe('version conflict');
    const invalid = page.generate({ id: 'draft-old', version: 3 });
    http.expectOne('/api/draft-generations').flush({ error: 'typed rejection' });
    await invalid;
    expect(page.error()).toContain('typed rejection');
    const reload = page.reload();
    http.expectOne('/api/draft-generations').flush({
      generations: [
        {
          ...record,
          status: 'interrupted',
          output_version: null,
          error: 'Restarted',
          usage: { input: null, output: null, model_seconds: null },
        },
      ],
    });
    await reload;
    expect(page.records()[0].usage.input).toBeNull();
    const failed = page.reload();
    http.expectOne('/api/draft-generations').flush({}, { status: 503, statusText: 'Unavailable' });
    await failed;
    expect(page.error()).toContain('连接失败');
    http.verify();
  });
});
