import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { RequirementList } from './requirement-list';
import { vi } from 'vitest';

describe('Requirement list polling', () => {
  it('retains facts on failure, prevents overlapping reads and stops polling when closed', async () => {
    vi.useFakeTimers();
    TestBed.configureTestingModule({ imports: [RequirementList], providers: [provideHttpClient(), provideHttpClientTesting(), provideRouter([])] });
    const http = TestBed.inject(HttpTestingController);
    const fixture = TestBed.createComponent(RequirementList);
    const initial = http.expectOne('/api/multi/requirements');
    await fixture.componentInstance.reload();
    http.expectNone('/api/multi/requirements');
    initial.flush({ requirements: [{ id: 1, title: 'Saved', version: 1, state: 'Running' }] });
    await Promise.resolve();
    expect(fixture.componentInstance.items()[0].title).toBe('Saved');
    await vi.advanceTimersByTimeAsync(5000);
    http.expectOne('/api/multi/requirements').flush({}, { status: 503, statusText: 'Unavailable' });
    await Promise.resolve();
    expect(fixture.componentInstance.error()).toContain('陈旧');
    expect(fixture.componentInstance.items()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(5000);
    http.expectOne('/api/multi/requirements').flush({ requirements: [] });
    await Promise.resolve();
    expect(fixture.componentInstance.error()).toBe('');
    fixture.destroy();
    await vi.advanceTimersByTimeAsync(5000);
    http.verify();
    vi.useRealTimers();
  });
});
