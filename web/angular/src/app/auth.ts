import { HttpClient, HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http';
import { inject, Service, signal } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { catchError, firstValueFrom, throwError } from 'rxjs';
import {
  AuthCsrfResponse,
  AuthLoginResponse,
  AuthSessionResponse,
  AuthLogoutResponse,
} from './health-response';

// Deliberately only canonical local paths. Encoded separators/double decoding,
// backslashes and protocol-relative targets never reach Router navigation.
export function safeReturn(value: string | null): string {
  if (!value || !/^\/(?!\/)[A-Za-z0-9/_-]*(?:\?[A-Za-z0-9_=&-]*)?$/.test(value)) return '/';
  return value.startsWith('/login') ? '/' : value;
}
@Service()
export class Auth {
  private readonly http = inject(HttpClient);
  readonly username = signal<string | null>(null);
  readonly expired = signal(false);
  readonly csrf = signal('');
  async check(): Promise<boolean> {
    if (this.username()) return true;
    try {
      const session = await firstValueFrom(this.http.get<AuthSessionResponse>('/api/auth/session'));
      if (!session) return false;
      this.accept(session);
      return true;
    } catch {
      return false;
    }
  }
  async login(username: string, password: string) {
    const proof = await firstValueFrom(this.http.get<AuthCsrfResponse>('/api/auth/csrf'));
    this.csrf.set(proof.csrf_token);
    const result = await firstValueFrom(
      this.http.post<AuthLoginResponse>('/api/auth/login', { username, password }),
    );
    if (!result || !('csrf_token' in result)) throw new Error('用户名或密码错误');
    this.accept(result);
    this.expired.set(false);
  }
  async logout() {
    try {
      await firstValueFrom(this.http.post<AuthLogoutResponse>('/api/auth/logout', {}));
    } catch (error: unknown) {
      if (!(error instanceof HttpErrorResponse && error.status === 401)) throw error;
    }
    this.username.set(null);
    this.csrf.set('');
    // Keep unsubmitted page input in memory, just as on absolute expiry.
    this.expired.set(true);
  }
  expire() {
    this.username.set(null);
    this.csrf.set('');
    this.expired.set(true);
  }
  private accept(response: { username: string | null; csrf_token: string }) {
    this.username.set(response.username);
    this.csrf.set(response.csrf_token);
  }
}
export const authGuard: CanActivateFn = async (_route, state) => {
  const auth = inject(Auth);
  const router = inject(Router);
  return (
    (await auth.check()) ||
    router.createUrlTree(['/login'], { queryParams: { return: safeReturn(state.url) } })
  );
};
export const authInterceptor: HttpInterceptorFn = (request, next) => {
  const auth = inject(Auth);
  if (!request.url.startsWith('/api/')) return next(request);
  const write = !['GET', 'HEAD', 'OPTIONS'].includes(request.method);
  const secured = write
    ? request.clone({ setHeaders: { 'x-codexsymphony-csrf': auth.csrf() } })
    : request;
  return next(secured).pipe(
    catchError((error: unknown) => {
      if (
        error instanceof HttpErrorResponse &&
        error.status === 401 &&
        !request.url.startsWith('/api/auth/')
      )
        auth.expire();
      return throwError(() => error);
    }),
  );
};
