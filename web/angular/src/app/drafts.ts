import { HttpErrorResponse } from '@angular/common/http';
import { Component, computed, inject, signal } from '@angular/core';
import { FormField, form } from '@angular/forms/signals';
import { RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { firstValueFrom } from 'rxjs';
import { DraftsApi } from './drafts-api';
import { GetDraftResponse, ListDraftsResponse } from './health-response';
import { draftExample } from './drafts-example';
type Draft = Extract<GetDraftResponse, { id: string }>;
@Component({
  imports: [FormField, RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule],
  selector: 'app-drafts',
  styleUrl: './drafts.scss',
  templateUrl: './drafts.html',
})
export class Drafts {
  private readonly api = inject(DraftsApi);
  readonly model = signal({ format: 'json' as 'json' | 'markdown', label: '', text: '' });
  readonly fields = form(this.model);
  readonly selected = signal<Draft | undefined>(undefined);
  readonly items = signal<ListDraftsResponse['drafts']>([]);
  readonly children = computed(() =>
    [...(this.selected()?.document.children ?? [])].sort((a, b) => a.order - b.order),
  );
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  constructor() {
    void this.reload();
  }
  async reload() {
    await this.perform(async () => {
      this.items.set((await firstValueFrom(this.api.list())).drafts);
    });
  }
  async open(id: string) {
    await this.perform(async () => {
      const result = await firstValueFrom(this.api.read(id));
      if ('error' in result) throw new Error(result.error);
      this.selected.set(result);
      this.model.set(result.source);
      this.message.set(`已读回 Draft 版本 ${result.version}`);
    });
  }
  newDraft() {
    this.selected.set(undefined);
    this.model.set({ format: 'json', label: '', text: '' });
    this.error.set('');
    this.message.set('新草稿尚未保存');
  }
  example() {
    this.model.update((value) => {
      return { ...value, format: 'json', text: draftExample };
    });
    this.message.set('样例仅供编辑：请填写来源并按已登记仓库 ID 调整 repository_id。');
  }
  async save() {
    await this.perform(async () => {
      const current = this.selected();
      const body = { version: current?.version ?? 0, source: this.model() };
      const result = await firstValueFrom(
        current ? this.api.update(current.id, body) : this.api.create(body),
      );
      if ('error' in result) throw new Error(result.error);
      this.selected.set(result);
      this.message.set(`Draft 已保存 · 版本 ${result.version} · 未授权执行`);
      this.items.set((await firstValueFrom(this.api.list())).drafts);
    });
  }
  private async perform(action: () => Promise<void>) {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await action();
    } catch (error: unknown) {
      this.error.set(error instanceof HttpErrorResponse ? this.httpError(error) : String(error));
    } finally {
      this.busy.set(false);
    }
  }
  private httpError(error: HttpErrorResponse): string {
    if (error.status === 409)
      return '版本冲突：本地输入已保留。请先复制输入，再重新打开已保存草稿并合并修改。';
    const body: unknown = error.error;
    if (typeof body === 'object' && body !== null && 'error' in body) return String(body.error);
    return '请求失败；输入已保留，请检查连接并重试。';
  }
}
