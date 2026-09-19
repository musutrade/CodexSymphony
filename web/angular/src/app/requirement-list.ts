import { Component, DestroyRef, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom, interval } from 'rxjs';
import { RequirementsApi } from './requirements/requirements-api';
import { ListRequirementsResponse } from './health-response';
@Component({
  selector: 'app-requirement-list',
  imports: [RouterLink, MatButtonModule],
  templateUrl: './requirement-list.html',
})
export class RequirementList {
  private readonly api = inject(RequirementsApi);
  readonly items = signal<ListRequirementsResponse['requirements']>([]);
  readonly loading = signal(true);
  readonly error = signal('');
  private reading = false;
  constructor() {
    void this.reload();
    interval(5000)
      .pipe(takeUntilDestroyed(inject(DestroyRef)))
      .subscribe(() => void this.reload());
  }
  async reload() {
    if (this.reading) return;
    this.reading = true;
    try {
      this.items.set((await firstValueFrom(this.api.list())).requirements);
      this.error.set('');
    } catch {
      this.error.set('连接中断，列表可能陈旧。后台继续运行；连接恢复后自动刷新。');
    } finally {
      this.reading = false;
      this.loading.set(false);
    }
  }
}
