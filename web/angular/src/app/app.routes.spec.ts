import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { routes } from './app.routes';
import { App } from './app';

describe('Workspace navigation', () => {
  it('links both pages and announces the current page', async () => {
    TestBed.configureTestingModule({
      imports: [App],
      providers: [provideRouter([{ path: '', children: [] }, { path: 'requirements', children: [] }])],
    });
    const fixture = TestBed.createComponent(App);
    const router = TestBed.inject(Router);
    fixture.detectChanges();
    await router.navigateByUrl('/');
    await fixture.whenStable();
    const element: HTMLElement = fixture.nativeElement;
    expect(element.querySelector('nav a[href="/"]')?.getAttribute('aria-current')).toBe('page');
    const requirements = element.querySelector<HTMLAnchorElement>('nav a[href="/requirements"]');
    expect(requirements?.textContent?.trim()).toBe('需求工作台');
    requirements?.click();
    await fixture.whenStable();
    expect(router.url).toBe('/requirements');
    expect(requirements?.getAttribute('aria-current')).toBe('page');
    expect(element.querySelector('nav a[href="/"]')?.hasAttribute('aria-current')).toBe(false);
  });
});

describe('Lazy requirement route', () => {
  it('opens the browser entry and starts real API reads', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    });
    const harness = await RouterTestingHarness.create('/requirements');
    const http = TestBed.inject(HttpTestingController);
    http
      .expectOne('/api/repository')
      .flush({
        repositories: [],
        deployment_network: [],
        network_status: 'not_configured',
        runtime_ready: false,
        repository_ready: false,
      });
    http.expectOne('/api/requirements').flush({ requirements: [] });
    await harness.fixture.whenStable();
    expect(harness.routeNativeElement?.textContent).toContain('需求工作台');
    http.verify();
  });
});
