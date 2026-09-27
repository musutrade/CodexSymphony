import { JsonPipe } from '@angular/common';
import { Component, inject, Input, OnChanges, signal } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom } from 'rxjs';
import { LifecycleApi } from './lifecycle-api';
import { GetLifecycleHistoryResponse, GetRequirementModelsResponse } from './health-response';
type Event = GetLifecycleHistoryResponse['events'][number];

@Component({
  imports: [JsonPipe, MatButtonModule],
  selector: 'app-lifecycle-panel',
  styleUrl: './lifecycle-panel.scss',
  templateUrl: './lifecycle-panel.html',
})
export class LifecyclePanel implements OnChanges {
  private readonly api = inject(LifecycleApi);
  @Input() requirementId = 0;
  @Input() version = 0;
  @Input() disabled = false;
  readonly events = signal<Event[]>([]);
  readonly models = signal<GetRequirementModelsResponse['runs']>([]);
  readonly error = signal('');
  readonly message = signal('');
  readonly busy = signal(false);
  readonly more = signal(false);
  private sequence = 0;
  ngOnChanges() {
    void this.refresh();
  }
  async refresh() {
    const sequence = ++this.sequence;
    const id = this.requirementId;
    try {
      const [history, models] = await Promise.all([
        firstValueFrom(this.api.events(id, 0)),
        firstValueFrom(this.api.models(id)),
      ]);
      if (sequence !== this.sequence) return;
      this.events.set(history.events);
      this.more.set(history.events.length === 100);
      this.models.set(models.runs);
      this.error.set('');
    } catch {
      if (sequence === this.sequence)
        this.error.set('生命周期或模型事实暂不可读。保留的内容可能陈旧，补投已停用。');
    }
  }
  async next() {
    if (this.busy() || this.error()) return;
    const sequence = this.sequence;
    this.busy.set(true);
    try {
      const after = this.events().at(-1)?.sequence ?? 0;
      const result = await firstValueFrom(this.api.events(this.requirementId, after));
      if (sequence !== this.sequence) return;
      this.events.update((events) => [...events, ...result.events]);
      this.more.set(result.events.length === 100);
    } catch {
      this.error.set('后续生命周期记录暂不可读，请刷新后继续。');
    } finally {
      this.busy.set(false);
    }
  }
  async replay(event: Event, plugin: string) {
    if (this.busy() || this.disabled || this.error() || !event.event_id || !plugin) return;
    this.busy.set(true);
    try {
      const result = await firstValueFrom(
        this.api.replay(this.requirementId, { event_id: event.event_id, plugin_id: plugin }),
      );
      this.message.set(
        result.accepted
          ? '有限补投已接受，尚未报告渠道发送；业务执行不会重跑。'
          : '补投未接受：请核对次数、截止时间或当前仓库授权。',
      );
    } catch {
      this.message.set('补投未保存或响应未知。请核对持久记录，不会自动重放。');
    } finally {
      await this.refresh();
      this.busy.set(false);
    }
  }
}
