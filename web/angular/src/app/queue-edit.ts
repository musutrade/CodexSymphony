import { JsonPipe } from '@angular/common';
import { Component, computed, inject, signal } from '@angular/core';
import { HttpErrorResponse } from '@angular/common/http';
import { FormField, form } from '@angular/forms/signals';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom } from 'rxjs';
import { GroupReviewApi } from './group-review-api';
import { GroupView, Review } from './group-review-model';
import { QUEUE_EDIT_CONTEXT } from './queue-edit-context';
import { EditGroupQueueRequest } from './health-response';
@Component({
  selector: 'app-queue-edit',
  imports: [JsonPipe, FormField, MatButtonModule],
  templateUrl: './queue-edit.html',
  styles: `
    textarea {
      width: 100%;
      box-sizing: border-box;
      min-height: 12rem;
    }
    pre {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }
    .ui-actions {
      flex-wrap: wrap;
    }
  `,
})
export class QueueEdit {
  private readonly context = inject(QUEUE_EDIT_CONTEXT);
  readonly view = computed(() => {
    return this.context.view()!;
  });
  private readonly api = inject(GroupReviewApi);
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  readonly editing = signal(false);
  readonly model = signal({ document: '', review: '' });
  readonly fields = form(this.model);
  readonly ordered = computed(() =>
    [...this.view().document.children].sort((a, b) => a.order - b.order),
  );
  begin() {
    const current = this.view();
    const document = current.pending_edit?.document ?? current.document;
    const review = structuredClone(current.pending_edit?.review ?? current.review) as Review;
    review.parent_revision = current.draft_revision + 1;
    review.items = review.items.map((item) => {
      return { ...item, revision: review.parent_revision };
    });
    review.coverage = review.coverage.map((mapping) => {
      return {
        ...mapping,
        child_revision: review.parent_revision,
      };
    });
    this.model.set({
      document: JSON.stringify(document, null, 2),
      review: JSON.stringify(review, null, 2),
    });
    this.editing.set(true);
  }
  async move(index: number, delta: number) {
    const order = this.ordered().map((child) => {
      return child.id;
    });
    [order[index], order[index + delta]] = [order[index + delta], order[index]];
    await this.perform({ kind: 'reorder', order });
  }
  async propose() {
    try {
      const document = JSON.parse(this.model().document) as GroupView['document'];
      const review = JSON.parse(this.model().review) as Review;
      await this.perform({ kind: 'propose', document, review });
    } catch {
      this.error.set('JSON 格式无效，请修正后再保存；本地输入已保留。');
    }
  }
  async approve() {
    const pending = this.view().pending_edit;
    if (pending) await this.perform({ kind: 'approve', edit_version: pending.version });
  }
  private async perform(change: EditGroupQueueRequest['change']) {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const current = this.view();
      const result = await firstValueFrom(
        this.api.edit(current.draft_id, {
          request_id: crypto.randomUUID(),
          version: current.queue?.version ?? 0,
          change,
        }),
      );
      if ('error' in result) throw new Error(result.error);
      this.message.set(
        `队列版本 ${result.version} 已保存。变化项：${result.affected.join('、') || '无内容变化'}。`,
      );
      this.editing.set(false);
      await this.context.reload();
    } catch (error: unknown) {
      const message = queueError(error);
      this.error.set(
        `操作未生效：${message}。版本冲突或已领取时，请保留输入并重新读取队列；运行项先使用停止与保全控制。`,
      );
    } finally {
      this.busy.set(false);
    }
  }
}

function queueError(error: unknown) {
  const detail: unknown = error instanceof HttpErrorResponse ? error.error : error;
  return typeof detail === 'object' && detail !== null && 'error' in detail
    ? String(detail.error)
    : String(detail);
}
