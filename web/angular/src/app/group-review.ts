import { JsonPipe } from '@angular/common';
import { Component, computed, inject, signal } from '@angular/core';
import { HttpErrorResponse } from '@angular/common/http';
import { FormField, form } from '@angular/forms/signals';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom } from 'rxjs';
import { GroupReviewApi } from './group-review-api';
import {
  GroupView,
  Review,
  editableReview,
  initialReview,
  totalBudget,
  zeroBudget,
} from './group-review-model';
@Component({
  selector: 'app-group-review',
  imports: [JsonPipe, FormField, RouterLink, MatButtonModule],
  templateUrl: './group-review.html',
  styleUrl: './group-review.scss',
})
export class GroupReview {
  private readonly api = inject(GroupReviewApi);
  private readonly id = inject(ActivatedRoute).snapshot.paramMap.get('id') ?? '';
  readonly view = signal<GroupView | undefined>(undefined);
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  readonly model = signal(
    editableReview({
      parent_revision: 0,
      full_chain_acs: [],
      coverage: [],
      items: [],
      group_budget: null,
      semantic_review: '',
    }),
  );
  readonly fields = form(this.model);
  readonly total = computed(() =>
    this.model().automatic_budget ? totalBudget(this.model().items) : this.model().group_budget,
  );
  private readonly saved = signal('');
  private requestId = '';
  readonly clean = computed(() => JSON.stringify(this.payload()) === this.saved());
  constructor() {
    void this.reload();
  }
  payload(): Review {
    const { automatic_budget, ...review } = this.model();
    return { ...review, group_budget: automatic_budget ? null : review.group_budget };
  }
  private apply(view: GroupView) {
    this.view.set(view);
    this.model.set(editableReview(view.review ?? initialReview(view)));
    this.saved.set(JSON.stringify(this.payload()));
    this.requestId = crypto.randomUUID();
  }
  async reload() {
    await this.perform(async () => {
      const result = await firstValueFrom(this.api.read(this.id));
      if ('error' in result) throw new Error(result.error);
      this.apply(result);
      this.message.set('已读回持久评审；请核对精确版本、覆盖语义与预算。');
    });
  }
  resetForRevision() {
    const current = this.view();
    if (current) {
      this.model.set(editableReview(initialReview(current)));
      this.message.set('已按当前版本重建待评审表单；尚未保存或授权。');
    }
  }
  fullChain(id: string, event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    this.model.update((m) => {
      return {
        ...m,
        full_chain_acs: checked
          ? [...m.full_chain_acs, id]
          : m.full_chain_acs.filter((a) => a !== id),
      };
    });
  }
  addMapping(parent: string) {
    this.model.update((m) => {
      return {
        ...m,
        coverage: [
          ...m.coverage,
          {
            parent_ac: parent,
            child_id: '',
            child_revision: m.parent_revision,
            child_ac: '',
            step_id: '',
          },
        ],
      };
    });
  }
  removeMapping(index: number) {
    this.model.update((m) => {
      return { ...m, coverage: m.coverage.filter((_, i) => i !== index) };
    });
  }
  budget(item: string) {
    return (
      this.view()?.budgets.find((b) => b.item_id === item) ?? {
        used: zeroBudget(),
        reserved: zeroBudget(),
      }
    );
  }
  async save() {
    await this.perform(async () => {
      const current = this.view();
      if (!current) return;
      const result = await firstValueFrom(
        this.api.save(this.id, {
          version: current.version,
          draft_revision: current.draft_revision,
          review: this.payload(),
        }),
      );
      if ('error' in result) throw new Error(result.error);
      this.apply(result);
      this.message.set('评审版本已保存；尚未确认本版本授权。');
    });
  }
  async confirm() {
    await this.perform(async () => {
      const current = this.view();
      if (!current) return;
      const result = await firstValueFrom(
        this.api.confirm(this.id, {
          request_id: this.requestId,
          version: current.version,
          draft_revision: current.draft_revision,
        }),
      );
      if ('error' in result) throw new Error(result.error);
      this.message.set(
        `授权 ${result.authorization_id} 已保存 / 等待调度能力。不是验收完成；无需逐项 Start。`,
      );
      const latest = await firstValueFrom(this.api.read(this.id));
      if ('error' in latest) throw new Error(latest.error);
      this.apply(latest);
    });
  }
  private async perform(action: () => Promise<void>) {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      await action();
    } catch (error: unknown) {
      const detail: unknown = error instanceof HttpErrorResponse ? error.error : error;
      this.error.set(
        typeof detail === 'object' && detail !== null && 'error' in detail
          ? String(detail.error)
          : String(detail),
      );
    } finally {
      this.busy.set(false);
    }
  }
}
