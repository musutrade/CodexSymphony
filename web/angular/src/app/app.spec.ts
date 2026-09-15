import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { HealthPage as App } from './health-page';

describe('Health overview', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [App],
      providers: [provideHttpClient(), provideHttpClientTesting()],
    });
  });
  afterEach(() => TestBed.inject(HttpTestingController).verify());

  it('shows confirmed service and database health', async () => {
    const fixture = TestBed.createComponent(App);
    TestBed.tick();
    TestBed.inject(HttpTestingController)
      .expectOne('/api/health')
      .flush({ status: 'ok', database: 'ok' });
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('[role="status"]').textContent).toContain(
      '连接正常',
    );
    expect(fixture.nativeElement.querySelectorAll('h1').length).toBe(1);
  });

  it('shows an unavailable connection and allows a fresh check', async () => {
    const fixture = TestBed.createComponent(App);
    TestBed.tick();
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/health').flush({}, { status: 503, statusText: 'Unavailable' });
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('暂时无法连接');
    fixture.nativeElement.querySelector('button').click();
    TestBed.tick();
    http.expectOne('/api/health').flush({ status: 'ok', database: 'ok' });
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('连接正常');
  });

  it('does not claim health from an incomplete response', async () => {
    const fixture = TestBed.createComponent(App);
    TestBed.tick();
    TestBed.inject(HttpTestingController).expectOne('/api/health').flush({ status: 'ok' });
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('暂时无法连接');
  });
});
