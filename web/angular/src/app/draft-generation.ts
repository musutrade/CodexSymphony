import { HttpClient, HttpErrorResponse } from '@angular/common/http';
import { Service, inject, signal } from '@angular/core';
import { form } from '@angular/forms/signals';
import { firstValueFrom } from 'rxjs';

import { GenerateDraftResponse, ListGenerationsResponse } from './health-response';
export type GenerationRecord = Extract<GenerateDraftResponse, { id: string }>;
@Service()
export class DraftGeneration {
  private readonly http = inject(HttpClient);
  readonly model = signal({ label: '', text: '' });
  readonly fields = form(this.model);
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  readonly records = signal<GenerationRecord[]>([]);
  private submission: { identity: string; requestId: string } | undefined;
  async generate(target?: { id: string; version: number }) {
    await this.perform(async () => {
      const body = { ...this.model(), draft_id: target?.id ?? null, version: target?.version ?? 0 };
      const identity = JSON.stringify(body);
      if (this.submission?.identity !== identity)
        this.submission = { identity, requestId: crypto.randomUUID() };
      const record = await firstValueFrom(
        this.http.post<GenerateDraftResponse>(
          '/api/draft-generations',
          { ...body, request_id: this.submission.requestId },
          { headers: { 'x-codexsymphony-csrf': '1' } },
        ),
      );
      if (!('id' in record)) throw new Error(record.error);
      this.records.set([record]);
      this.message.set('生成请求已持久化。刷新记录查看结果；重复相同输入不会再次调用模型。');
    });
  }
  async reload() {
    await this.perform(async () => {
      const result = await firstValueFrom(
        this.http.get<ListGenerationsResponse>('/api/draft-generations'),
      );
      this.records.set(result.generations);
      this.message.set('已读回生成状态；原编辑区保持不变。');
    });
  }
  stateLabel(state: GenerationRecord['status']) {
    return {
      running: '生成中',
      succeeded: '草稿已保存，未授权',
      failed: '生成失败',
      interrupted: '服务重启后中断',
      conflict: '版本冲突，用户修改已保留',
    }[state];
  }
  private async perform(action: () => Promise<void>) {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await action();
    } catch (error: unknown) {
      this.error.set(error instanceof HttpErrorResponse ? this.messageFor(error) : String(error));
    } finally {
      this.busy.set(false);
    }
  }
  private messageFor(error: HttpErrorResponse): string {
    const body: unknown = error.error;
    if (typeof body === 'object' && body !== null && 'error' in body) return String(body.error);
    return '连接失败，输入和请求 ID 已保留；重试相同输入不会重复消费。';
  }
}
