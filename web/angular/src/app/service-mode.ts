import { HttpErrorResponse, HttpInterceptorFn } from '@angular/common/http';
import { inject, Service, signal } from '@angular/core';
import { tap } from 'rxjs';

@Service()
export class ServiceMode {
  readonly observationOnly = signal(false);

  accept(value: string | null) {
    if (value === 'observation-only') this.observationOnly.set(true);
    if (value === 'normal') this.observationOnly.set(false);
  }
}

export const serviceModeInterceptor: HttpInterceptorFn = (request, next) => {
  const mode = inject(ServiceMode);
  if (!request.url.startsWith('/api/')) return next(request);
  return next(request).pipe(
    tap({
      next: (event) => {
        if ('body' in event && 'headers' in event)
          mode.accept(event.headers.get('x-codexsymphony-service-mode'));
      },
      error: (error: unknown) => {
        if (error instanceof HttpErrorResponse)
          mode.accept(error.headers.get('x-codexsymphony-service-mode'));
      },
    }),
  );
};
