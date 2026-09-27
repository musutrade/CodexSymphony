import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ActivatedRoute, convertToParamMap } from '@angular/router';
import { BehaviorSubject } from 'rxjs';
import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';
import { DiagnosticPanel } from './diagnostic-panel';
import { GetDiagnosticsResponse } from '../health-response';
type Artifact = Extract<GetDiagnosticsResponse, { artifacts: unknown }>['artifacts'][number];
const text = 'FAIL-FIRST\n测量 failure\nFAIL-LAST\n';
const artifact: Artifact = {
  artifact_id: 'identity',
  purpose: 'report.md',
  media_type: 'text/markdown',
  availability: 'available',
  reason: null,
  original_bytes: new TextEncoder().encode(text).byteLength,
  retained_bytes: new TextEncoder().encode(text).byteLength,
  raw_sha256: 'raw',
  export_bytes: new TextEncoder().encode(text).byteLength,
  export_sha256: '35139bed1973bb5ac04527cb7ba9989db60cbdc8391c0c6b9086f108530f3166',
  expires_at: 100,
  binding: {
    identity: {
      protocol_version: 2,
      requirement_id: 1,
      revision: 1,
      run_id: 'run',
      resource_id: 'resource',
      invocation_id: 'invocation',
      attempt: 1,
      config_id: 'config',
    },
    phase: 'validation',
    candidate: { sha: 'commit', tree: 'tree', immutable: true },
    validation_id: 'validation',
    generation: 1,
    implementation_digest: 'implementation',
    environment_digest: 'environment',
    policy_digest: 'policy',
  },
};
const page = (offset = 0, end = true, value = text) => ({
  artifact,
  offset,
  next: end ? artifact.export_bytes : offset + new TextEncoder().encode(value).byteLength,
  end,
  unit: 'bytes',
  text: value,
});
describe('Retained diagnostics', () => {
  let http: HttpTestingController;
  let params: BehaviorSubject<ReturnType<typeof convertToParamMap>>;
  async function setup() {
    params = new BehaviorSubject(convertToParamMap({ id: '1' }));
    TestBed.configureTestingModule({
      imports: [DiagnosticPanel],
      providers: [
        provideHttpClient(),
        provideHttpClientTesting(),
        {
          provide: ActivatedRoute,
          useValue: { snapshot: { paramMap: params.value }, paramMap: params },
        },
      ],
    });
    http = TestBed.inject(HttpTestingController);
    const fixture = TestBed.createComponent(DiagnosticPanel);

    await vi.waitFor(() =>
      http
        .expectOne('/api/requirements/1/diagnostics/0')
        .flush({ artifacts: [artifact], next: null }),
    );
    await fixture.whenStable();
    return fixture;
  }
  afterEach(() => {
    http.verify();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
  it('shows exact identity, availability and byte ranges and reads a later segment', async () => {
    const fixture = await setup();
    const component = fixture.componentInstance;
    expect(fixture.nativeElement.textContent).toContain('commit');
    for (const state of ['available', 'partial', 'missing', 'expired', 'corrupt'] as const)
      expect(component.state(state)).toBeTruthy();
    expect(component.readable({ ...artifact, availability: 'expired' })).toBe(false);
    const first = component.read(artifact, 0);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192')
      .flush(page(0, false, 'FAIL-FIRST\n'));
    await first;
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('下一段');
    const second = component.read(artifact, 11);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/11/8192')
      .flush(page(11, true, text.slice(11)));
    await second;
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('pre').textContent).toContain('FAIL-LAST');
    const more = component.load(8);
    http.expectOne('/api/requirements/1/diagnostics/8').flush({
      artifacts: [
        {
          ...artifact,
          artifact_id: 'missing',
          availability: 'missing',
          reason: 'not generated',
          binding: { ...artifact.binding, candidate: null },
        },
      ],
      next: 9,
    });
    await more;
    await fixture.whenStable();
    expect(component.artifacts()).toHaveLength(2);
    expect(fixture.nativeElement.textContent).toContain('not generated');
  });
  it('handles unavailable lists and content without rendering diagnostic HTML', async () => {
    const fixture = await setup();
    const component = fixture.componentInstance;
    let pending = component.load(0);
    http.expectOne('/api/requirements/1/diagnostics/0').flush({ error: 'revoked' });
    await pending;
    expect(component.error()).toContain('暂不可读');
    pending = component.read(artifact, 0);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192')
      .flush({ error: 'expired' });
    await pending;
    expect(component.error()).toContain('权限');
    pending = component.read(artifact, 0);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192')
      .flush(page(0, true, '<script>alert(1)</script>'));
    await pending;
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('script')).toBeNull();
    expect(fixture.nativeElement.querySelector('pre').textContent).toContain('<script>');
  });
  it('rejects identity changes, impossible ranges and stalled pagination', async () => {
    const fixture = await setup();
    const component = fixture.componentInstance;
    for (const invalid of [
      { ...page(), artifact: { ...artifact, artifact_id: 'forged' } },
      { ...page(), next: artifact.export_bytes + 1 },
      { ...page(), next: 0, end: false },
      { ...page(), end: false },
      { ...page(), offset: 1 },
    ]) {
      const pending = component.read(artifact, 0);
      http.expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192').flush(invalid);
      await pending;
      expect(component.error()).toContain('权限');
    }
  });
  it('downloads every segment and verifies the export hash before creating a file', async () => {
    const fixture = await setup();
    const component = fixture.componentInstance;
    const { webcrypto } = await vi.importActual<{ webcrypto: Crypto }>('node:crypto');
    vi.stubGlobal('crypto', webcrypto);
    const created = vi.fn(() => 'blob:fixture');
    const revoked = vi.fn();
    Object.defineProperty(URL, 'createObjectURL', { value: created, configurable: true });
    Object.defineProperty(URL, 'revokeObjectURL', { value: revoked, configurable: true });
    const clicked = vi
      .spyOn(HTMLAnchorElement.prototype, 'click')
      .mockImplementation(() => undefined);
    const pending = component.download(artifact);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192')
      .flush(page(0, false, 'FAIL-FIRST\n'));
    await vi.waitFor(() =>
      http
        .expectOne('/api/requirements/1/diagnostic-artifacts/identity/11/8192')
        .flush(page(11, true, text.slice(11))),
    );
    await pending;
    expect(created).toHaveBeenCalledOnce();
    expect(clicked).toHaveBeenCalledOnce();
    expect(revoked).toHaveBeenCalledWith('blob:fixture');
    const bad = component.download(artifact);
    http
      .expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192')
      .flush(page(0, true, 'tampered'));
    await bad;
    expect(component.error()).toContain('摘要');
    expect(clicked).toHaveBeenCalledOnce();
  });
  it('discards stale reads after the selected Requirement changes', async () => {
    const fixture = await setup();
    const component = fixture.componentInstance;
    const stale = component.read(artifact, 0);
    const request = http.expectOne('/api/requirements/1/diagnostic-artifacts/identity/0/8192');
    params.next(convertToParamMap({ id: '2' }));
    await vi.waitFor(() =>
      http.expectOne('/api/requirements/2/diagnostics/0').flush({ artifacts: [], next: null }),
    );
    request.flush(page());
    await stale;
    await fixture.whenStable();
    expect(component.chunk()).toBeNull();
    expect(component.artifacts()).toEqual([]);
    const old = component.load(0);
    const oldRequest = http.expectOne('/api/requirements/2/diagnostics/0');
    const fresh = component.load(0);
    http.expectOne('/api/requirements/2/diagnostics/0').flush({ artifacts: [], next: null });
    oldRequest.flush({ error: 'old' });
    await Promise.all([old, fresh]);
    expect(component.error()).toBe('');
  });
});
