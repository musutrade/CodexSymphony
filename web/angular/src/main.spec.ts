import { bootstrapApplication } from '@angular/platform-browser';
import { vi } from 'vitest';

vi.mock('@angular/platform-browser', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@angular/platform-browser')>()),
  bootstrapApplication: vi.fn(),
}));

describe('Application startup', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.resetModules();
  });

  it('boots the application once', async () => {
    vi.mocked(bootstrapApplication).mockResolvedValue(
      {} as Awaited<ReturnType<typeof bootstrapApplication>>,
    );
    await import('./main');
    expect(bootstrapApplication).toHaveBeenCalledOnce();
  });

  it('reports a failed bootstrap', async () => {
    const failure = new Error('startup failed');
    const report = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    vi.mocked(bootstrapApplication).mockRejectedValue(failure);
    await import('./main');
    await Promise.resolve();
    expect(report).toHaveBeenCalledWith(failure);
  });
});
