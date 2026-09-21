import {
  provideHttpClient,
  withInterceptors,
  HttpClient,
  HttpErrorResponse,
} from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import {
  provideRouter,
  Router,
  ActivatedRouteSnapshot,
  RouterStateSnapshot,
} from '@angular/router';
import { Auth, authGuard, authInterceptor, safeReturn } from './auth';

const session = { username: 'operator', csrf_token: 'response-derived-proof' };
describe('platform authentication', () => {
  beforeEach(() =>
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([authInterceptor])),
        provideHttpClientTesting(),
      ],
    }),
  );
  afterEach(() => TestBed.inject(HttpTestingController).verify());
  it('only accepts canonical local return paths', () => {
    for (const path of [
      null,
      '',
      '//evil.test',
      'https://evil.test',
      '/%2f%2fevil',
      '/%252f',
      '/\\evil',
      '/login?return=evil',
      '/x#fragment',
      '/x\n',
    ])
      expect(safeReturn(path)).toBe('/');
    expect(safeReturn('/drafts/draft-1/review?version=2')).toBe('/drafts/draft-1/review?version=2');
  });
  it('bootstraps a proof before login, rotates it, and revokes on logout', async () => {
    const auth = TestBed.inject(Auth),
      http = TestBed.inject(HttpTestingController);
    const login = auth.login('operator', 'synthetic');
    http.expectOne('/api/auth/csrf').flush({ username: null, csrf_token: 'anonymous-proof' });
    await Promise.resolve();
    const request = http.expectOne('/api/auth/login');
    expect(request.request.headers.get('x-codexsymphony-csrf')).toBe('anonymous-proof');
    expect(request.request.body).toEqual({ username: 'operator', password: 'synthetic' });
    request.flush(session);
    await login;
    expect(auth.username()).toBe('operator');
    expect(await auth.check()).toBe(true);
    const logout = auth.logout();
    const revoked = http.expectOne('/api/auth/logout');
    expect(revoked.request.headers.get('x-codexsymphony-csrf')).toBe(session.csrf_token);
    revoked.flush(null, { status: 204, statusText: 'No Content' });
    await logout;
    expect(auth.username()).toBeNull();
    expect(auth.csrf()).toBe('');
    expect(auth.expired()).toBe(true);
  });
  it('preserves failures and never retries a business write on 401', async () => {
    const auth = TestBed.inject(Auth),
      http = TestBed.inject(HttpTestingController);
    auth.username.set('operator');
    auth.csrf.set('current');
    let failed = false;
    TestBed.inject(HttpClient)
      .post('/api/drafts', { text: 'keep this' })
      .subscribe({
        error: (e: HttpErrorResponse) => {
          failed = e.status === 401;
        },
      });
    http.expectOne('/api/drafts').flush(null, { status: 401, statusText: 'Unauthorized' });
    expect(failed).toBe(true);
    expect(auth.expired()).toBe(true);
    expect(auth.username()).toBeNull();
    http.expectNone('/api/drafts');
    TestBed.inject(HttpClient).get('/assets/example.json').subscribe();
    http.expectOne('/assets/example.json').flush({});
  });
  it('restores a session or redirects safely without showing an expiry overlay at first visit', async () => {
    const auth = TestBed.inject(Auth),
      http = TestBed.inject(HttpTestingController);
    const check = auth.check();
    http.expectOne('/api/auth/session').flush(session);
    expect(await check).toBe(true);
    auth.username.set(null);
    const guard = TestBed.runInInjectionContext(() =>
      authGuard({} as ActivatedRouteSnapshot, { url: '//evil.test' } as RouterStateSnapshot),
    );
    http.expectOne('/api/auth/session').flush(null, { status: 401, statusText: 'Unauthorized' });
    expect(
      TestBed.inject(Router).serializeUrl((await guard) as ReturnType<Router['parseUrl']>),
    ).toBe('/login?return=%2F');
    expect(auth.expired()).toBe(false);
  });
  it('keeps failed login separate from business expiry and accepts reauthentication without replay', async () => {
    const auth = TestBed.inject(Auth),
      http = TestBed.inject(HttpTestingController);
    const failed = auth.login('missing', 'wrong');
    const result = expect(failed).rejects.toBeDefined();
    http.expectOne('/api/auth/csrf').flush({ username: null, csrf_token: 'proof' });
    await Promise.resolve();
    http
      .expectOne('/api/auth/login')
      .flush({ message: '用户名或密码错误' }, { status: 401, statusText: 'Unauthorized' });
    await result;
    expect(auth.expired()).toBe(false);
    auth.expire();
    const restored = auth.login('operator', 'new');
    http.expectOne('/api/auth/csrf').flush({ username: null, csrf_token: 'fresh' });
    await Promise.resolve();
    http.expectOne('/api/auth/login').flush(session);
    await restored;
    expect(auth.expired()).toBe(false);
    http.expectNone('/api/drafts');
  });
});


describe('Already expired sign-out', () => {
  it('clears local identity and opens reauthentication on a backend 401', async () => {
    TestBed.configureTestingModule({providers: [provideHttpClient(withInterceptors([authInterceptor])), provideHttpClientTesting()]});
    const auth = TestBed.inject(Auth), http = TestBed.inject(HttpTestingController);
    auth.username.set('operator');
    const logout = auth.logout();
    http.expectOne('/api/auth/logout').flush(null, {status:401,statusText:'Unauthorized'});
    await logout;
    expect(auth.username()).toBeNull(); expect(auth.expired()).toBe(true);
    http.verify();
  });
});
