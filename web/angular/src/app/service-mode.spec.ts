import {
  HttpClient,
  HttpHeaderResponse,
  HttpHeaders,
  provideHttpClient,
  withInterceptors,
} from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { App } from './app';
import { ServiceMode, serviceModeInterceptor } from './service-mode';

describe('Service observation mode', () => {
  beforeEach(() =>
    TestBed.configureTestingModule({
      imports: [App],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([serviceModeInterceptor])),
        provideHttpClientTesting(),
      ],
    }),
  );
  afterEach(() => TestBed.inject(HttpTestingController).verify());

  it('announces restricted access from a real API response and clears on normal recovery', async () => {
    const fixture = TestBed.createComponent(App);
    const client = TestBed.inject(HttpClient),
      http = TestBed.inject(HttpTestingController);
    client.get('/api/health').subscribe();
    http
      .expectOne('/api/health')
      .flush({}, { headers: { 'x-codexsymphony-service-mode': 'observation-only' } });
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('[role="status"]').textContent).toContain(
      '当前仅可查看',
    );
    client.get('/api/health').subscribe();
    http
      .expectOne('/api/health')
      .flush({}, { headers: { 'x-codexsymphony-service-mode': 'normal' } });
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('[role="status"]')).toBeNull();
  });

  it('ignores sent and header-only events until the final response arrives', () => {
    const mode = TestBed.inject(ServiceMode),
      client = TestBed.inject(HttpClient),
      http = TestBed.inject(HttpTestingController);
    client.get('/api/health', { observe: 'events' }).subscribe();
    const pending = http.expectOne('/api/health');
    pending.event(
      new HttpHeaderResponse({
        headers: new HttpHeaders({
          'x-codexsymphony-service-mode': 'observation-only',
        }),
      }),
    );
    expect(mode.observationOnly()).toBe(false);
    pending.flush({}, { headers: { 'x-codexsymphony-service-mode': 'observation-only' } });
    expect(mode.observationOnly()).toBe(true);
  });

  it('keeps the restriction on missing headers and reports a denied write without replay', () => {
    const mode = TestBed.inject(ServiceMode),
      client = TestBed.inject(HttpClient),
      http = TestBed.inject(HttpTestingController);
    let errors = 0;
    client.post('/api/operator/resume', {}).subscribe({ error: () => errors++ });
    http.expectOne('/api/operator/resume').flush(
      {},
      {
        status: 503,
        statusText: 'Unavailable',
        headers: { 'x-codexsymphony-service-mode': 'observation-only' },
      },
    );
    expect(errors).toBe(1);
    expect(mode.observationOnly()).toBe(true);
    client.get('/api/health').subscribe();
    http.expectOne('/api/health').flush({});
    expect(mode.observationOnly()).toBe(true);
    client.get('/assets/example.json').subscribe();
    http
      .expectOne('/assets/example.json')
      .flush({}, { headers: { 'x-codexsymphony-service-mode': 'normal' } });
    expect(mode.observationOnly()).toBe(true);
    http.expectNone('/api/operator/resume');
  });
});
