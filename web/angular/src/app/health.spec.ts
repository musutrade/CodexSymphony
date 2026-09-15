import { ApplicationRef } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { Health } from './health';

describe('Health resource', () => {
  it('keeps a failed database probe unavailable', async () => {
    TestBed.configureTestingModule({ providers: [provideHttpClient(), provideHttpClientTesting()] });
    const health = TestBed.inject(Health);
    TestBed.tick();
    const http = TestBed.inject(HttpTestingController);
    http.expectOne('/api/health').flush({ status: 'unavailable', database: 'unavailable' });
    await TestBed.inject(ApplicationRef).whenStable();
    expect(health.response.value()?.database).toBe('unavailable');
    http.verify();
  });
});
