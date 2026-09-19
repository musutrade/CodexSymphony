import { Component, DestroyRef, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { FormControl, ReactiveFormsModule, Validators } from '@angular/forms';
import { MatInputModule } from '@angular/material/input';
import { firstValueFrom, interval } from 'rxjs';
import { OperationsApi } from './operations-api';
import { ControlOperationsRequest, GetOperationsResponse } from './health-response';

export type OperationDetail = Extract<GetOperationsResponse, { requirement: unknown }>;
type Question = OperationDetail['questions'][number];
@Component({
  selector: 'app-operations',
  imports: [RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, ReactiveFormsModule],
  templateUrl: './operations.html',
  styleUrl: './operations.scss',
})
export class Operations {
  private readonly api = inject(OperationsApi);
  private readonly route = inject(ActivatedRoute);
  private readonly destroy = inject(DestroyRef);
  readonly inbox = !this.route.snapshot.paramMap.has('id');
  readonly items = signal<OperationDetail[]>([]);
  readonly loading = signal(true);
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  readonly preview = signal('');
  readonly cancelId = signal<number | null>(null);
  readonly drafts = new Map<string, FormControl<string>>();
  readonly invalidQuestion = signal('');
  private pending = { fingerprint: '', key: '' };
  private readVersion = 0;
  constructor() {
    this.route.paramMap.pipe(takeUntilDestroyed(this.destroy)).subscribe(() => {
      this.items.set([]);
      this.preview.set('');
      this.cancelId.set(null);
      void this.reload();
    });
    interval(5000)
      .pipe(takeUntilDestroyed(this.destroy))
      .subscribe(() => {
        if (!this.busy()) void this.reload();
      });
  }
  async reload() {
    const readVersion = ++this.readVersion;
    try {
      const ids = this.inbox
        ? (await firstValueFrom(this.api.inbox())).requirement_ids
        : [Number(this.route.snapshot.paramMap.get('id'))];
      const items = await Promise.all(ids.map((id) => firstValueFrom(this.api.detail(id))));
      if (readVersion !== this.readVersion) return;
      this.items.set(items.map(requireDetail));
      this.error.set('');
    } catch {
      if (readVersion !== this.readVersion) return;
      this.error.set('无法读取最新状态。当前内容可能陈旧，操作已停用；连接恢复后会自动接续。');
    } finally {
      if (readVersion === this.readVersion) this.loading.set(false);
    }
  }
  async perform(action: () => Promise<unknown>) {
    if (this.busy()) return;
    this.busy.set(true);
    this.message.set('');
    try {
      await action();
      this.message.set('操作已保存。后台按当前授权与恢复条件继续处理。');
    } catch {
      this.message.set('操作未保存或状态已变化。已重新读取当前版本；请核对后重试。');
    } finally {
      await this.reload();
      this.busy.set(false);
    }
  }
  async control(item: OperationDetail, action: ControlOperationsRequest['action']) {
    const payload = { id: item.requirement.id, version: item.requirement.version, action };
    const fingerprint = JSON.stringify(payload);
    if (fingerprint !== this.pending.fingerprint)
      this.pending = { fingerprint, key: crypto.randomUUID() };
    await this.perform(() =>
      firstValueFrom(
        this.api.control(payload.id, {
          version: payload.version,
          action,
          request_id: this.pending.key,
        }),
      ),
    );
    this.cancelId.set(null);
  }
  answerControl(question: string, part: string) {
    const key = question + ':' + part;
    let control = this.drafts.get(key);
    if (!control) {
      control = new FormControl('', {
        nonNullable: true,
        validators: [Validators.required, Validators.pattern(/\S/)],
      });
      this.drafts.set(key, control);
    }
    return control;
  }
  async answer(question: Question) {
    const answers = question.questions.map((q) => {
      return {
        id: q.id,
        text: this.answerControl(question.id, q.id).value.trim(),
      };
    });
    if (answers.some((answer) => !answer.text)) {
      question.questions.forEach((q) => this.answerControl(question.id, q.id).markAsTouched());
      this.invalidQuestion.set(question.id);
      this.message.set('请填写每个问题的回答。');
      return;
    }
    this.invalidQuestion.set('');
    await this.perform(() =>
      firstValueFrom(this.api.answer(question.id, { version: question.version, answers })),
    );
  }
  async evidence(id: number, material: OperationDetail['materials'][number]) {
    await this.perform(async () => {
      const result = await firstValueFrom(this.api.evidence(id, material.run_id, material.channel));
      if (!('text' in result)) throw new Error('Evidence unavailable');
      this.preview.set(result.text);
    });
  }
  prLink(repository: string, number: number | null): string | null {
    return /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository) && number !== null && number > 0
      ? `https://github.com/${repository}/pull/${number}`
      : null;
  }
}

function requireDetail(value: GetOperationsResponse): OperationDetail {
  if (!('requirement' in value)) throw new Error('Requirement unavailable');
  return value;
}
