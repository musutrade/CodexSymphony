import { Component, effect, inject, EventEmitter, Input, Output, signal } from '@angular/core';
import { FormField, form, required } from '@angular/forms/signals';
import { MatButtonModule } from '@angular/material/button';
import { firstValueFrom } from 'rxjs';
import { LifecycleApi } from './lifecycle-api';
import { OperationDetail } from './operations';
import { GetExtensionRecoveryResponse, ResolveExtensionRecoveryRequest } from './health-response';
type Failure = GetExtensionRecoveryResponse['failures'][number];

@Component({
  imports: [FormField, MatButtonModule],
  selector: 'app-recovery-panel',
  styleUrl: './recovery-panel.scss',
  templateUrl: './recovery-panel.html',
})
export class RecoveryPanel {
  private readonly api = inject(LifecycleApi);
  protected readonly detail = signal<OperationDetail | null>(null);
  @Input() set item(value: OperationDetail) {
    this.detail.set(value);
  }
  @Input() disabled = false;
  @Output() readonly changed = new EventEmitter<void>();
  readonly failures = signal<Failure[]>([]);
  readonly error = signal('');
  readonly message = signal('');
  readonly busy = signal(false);
  readonly selected = signal<Failure | null>(null);
  readonly model = signal({
    kind: 'revalidate',
    reason: '',
    plan: '',
    policy: '',
    condition: '',
    instruction: '',
    paths: '',
    release: '',
  });
  readonly fields = form(this.model, (fields) => required(fields.reason));
  private sequence = 0;
  private pending = { fingerprint: '', key: '' };
  constructor() {
    effect(() => {
      const item = this.detail()!;
      if (item) void this.load(item.requirement.id);
    });
  }
  async load(id: number) {
    const sequence = ++this.sequence;
    try {
      const result = await firstValueFrom(this.api.recovery(id));
      if (sequence !== this.sequence) return;
      this.failures.set(result.failures);
      this.error.set('');
    } catch {
      if (sequence === this.sequence)
        this.error.set('恢复事实暂不可读，恢复操作已停用。输入仍保留。');
    }
  }
  eligible(failure: Failure) {
    const current = this.failures().find((entry) => entry.event_key === failure.event_key);
    if (!current || current.validation_id !== failure.validation_id) return false;
    return this.canRecover(current);
  }
  private canRecover(failure: Failure) {
    const task = this.detail()!;
    if (this.unavailable()) return false;
    if (failure.decision !== 'blocked' || failure.successor_validation) return false;
    if (failure.resolution && failure.resolution_state !== 'blocked') return false;
    return task.validations.some(
      (v) =>
        v.id === failure.validation_id &&
        v.revision === task.requirement.revision &&
        !v.superseded_by,
    );
  }
  private unavailable() {
    const r = this.detail()!.requirement;
    return this.disabled || !!this.error() || this.busy() || r.paused || r.cancel_requested;
  }
  choose(failure: Failure) {
    this.selected.set(failure);
    this.message.set('');
  }
  action(): ResolveExtensionRecoveryRequest['action'] {
    const m = this.model();
    if (!m.reason.trim()) throw new Error('请填写恢复理由。');
    if (m.kind === 'adapt_code') {
      return this.adaptation();
    }
    if (!/^[a-fA-F0-9]{64}$/.test(m.plan) || !m.condition.trim())
      throw new Error('请填写批准的计划摘要和已满足的恢复条件。');
    if (m.kind === 'revalidate_delivery') {
      if (!/^[a-fA-F0-9]{64}$/.test(m.policy)) throw new Error('请填写批准的交付策略摘要。');
      return {
        kind: 'revalidate_delivery',
        plan_digest: m.plan,
        policy_digest: m.policy,
        resume_condition: m.condition,
      };
    }
    return { kind: 'revalidate', plan_digest: m.plan, resume_condition: m.condition };
  }
  private adaptation(): ResolveExtensionRecoveryRequest['action'] {
    const m = this.model();
    const paths = m.paths
      .split('\n')
      .map((p) => p.trim())
      .filter(Boolean);
    if (!m.instruction.trim() || !m.release.trim() || !paths.length)
      throw new Error('请填写代码约束、允许路径和解除条件。');
    return {
      kind: 'adapt_code',
      constraints: [
        {
          id: 'operator-adaptation',
          version: '1',
          source: 'operator_review',
          reason: m.reason,
          instruction: m.instruction,
          paths,
          code_scope: 'specified_files',
          release_condition: m.release,
        },
      ],
    };
  }
  private payload(failure: Failure) {
    const r = this.detail()!.requirement;
    const payload = {
      version: r.version,
      revision: r.revision,
      validation_id: failure.validation_id ?? '',
      reason: this.model().reason,
      action: this.action(),
    };
    const fingerprint = JSON.stringify(payload);
    if (fingerprint !== this.pending.fingerprint)
      this.pending = { fingerprint, key: crypto.randomUUID() };
    return { ...payload, request_id: this.pending.key };
  }
  async submit() {
    const failure = this.selected();
    if (!failure || !this.eligible(failure)) return;
    try {
      const payload = this.payload(failure);
      this.busy.set(true);
      const result = await firstValueFrom(this.api.resolve(this.detail()!.requirement.id, payload));
      if ('error' in result || !result.accepted)
        throw new Error('恢复决定未接受。请刷新当前状态再核对。');
      this.message.set(
        result.started
          ? '后台已报告启动。请核对后续 Run 与验证记录。'
          : '恢复决定已保存，尚未报告会话启动。沿用原授权和累计预算，等待后台核对恢复条件。',
      );
      this.selected.set(null);
      this.changed.emit();
    } catch (error: unknown) {
      this.message.set(
        error instanceof Error
          ? error.message
          : '恢复未保存或响应未知。输入已保留，请刷新后核对，不会自动重放。',
      );
      this.changed.emit();
    } finally {
      this.busy.set(false);
    }
  }
}
