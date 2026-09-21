import { TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { Login } from './login';
import { Auth, authInterceptor } from '../auth';
import { App } from '../app';

describe('Login and sign-out interaction', () => {
  beforeEach(() =>
    TestBed.configureTestingModule({
      imports: [Login, App],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([authInterceptor])),
        provideHttpClientTesting(),
      ],
    }),
  );
  afterEach(() => TestBed.inject(HttpTestingController).verify());
  it('focuses the accessible form, supports a safe return and clears the password after login', async () => {
    const fixture = TestBed.createComponent(Login),
      page = fixture.componentInstance;
    await fixture.whenStable();
    expect(fixture.nativeElement.querySelector('input[type=password]').autocomplete).toBe(
      'current-password',
    );
    page.model.set({ username: 'operator', password: 'synthetic-password' });
    const submit = page.submit(new Event('submit'));
    await page.submit(new Event('submit')); // duplicate pending submission
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/auth/csrf').flush({ csrf_token: 'anonymous-proof', username: null });
    await Promise.resolve();
    http.expectOne('/api/auth/login').flush({ csrf_token: 'session-proof', username: 'operator' });
    await submit;
    expect(page.model().password).toBe('');
    expect(page.busy()).toBe(false);
    expect(TestBed.inject(Router).url).toBe('/');
  });
  it('reports failed and limited attempts, then resumes without navigating or replaying writes', async () => {
    TestBed.inject(Auth).expire();
    const fixture = TestBed.createComponent(Login),
      page = fixture.componentInstance;
    const http = TestBed.inject(HttpTestingController);
    for (const status of [401, 429, 200]) {
      const submit = page.submit(new Event('submit'));
      http.expectOne('/api/auth/csrf').flush({ csrf_token: 'proof', username: null });
      await Promise.resolve();
      const request = http.expectOne('/api/auth/login');
      if (status === 200) request.flush({ csrf_token: 'proof', username: 'operator' });
      else request.flush({}, { status, statusText: 'Rejected' });
      await submit;
      expect(page.busy()).toBe(false);
      if (status === 429) expect(page.error()).toContain('15 分钟');
      if (status === 401) expect(page.error()).toContain('用户名或密码错误');
    }
    http.expectNone('/api/drafts');
  });
  it('allows retry after a failed sign-out', async () => {
    const fixture = TestBed.createComponent(App),
      page = fixture.componentInstance;
    TestBed.inject(Auth).csrf.set('proof');
    const http = TestBed.inject(HttpTestingController);
    const failed = page.logout();
    http.expectOne('/api/auth/logout').flush({}, { status: 503, statusText: 'Offline' });
    await failed;
    expect(page.logoutError()).toContain('重试');
    const done = page.logout();
    http.expectOne('/api/auth/logout').flush(null, { status: 204, statusText: 'No Content' });
    await done;
    expect(TestBed.inject(Auth).expired()).toBe(true);
  });
});
