import { HttpErrorResponse } from '@angular/common/http';
import { RouterLink } from '@angular/router';
import { Component, inject, signal } from '@angular/core';
import { FormBuilder, ReactiveFormsModule, Validators } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatSelectModule } from '@angular/material/select';
import { MatInputModule } from '@angular/material/input';
import { firstValueFrom } from 'rxjs';
import {
  MultiCreateRequirementRequest,
  MultiGetRequirementResponse,
  MultiGetRepositoryResponse,
  MultiListRequirementsResponse,
} from '../health-response';
import { RequirementsApi } from './requirements-api';

type Requirement = Extract<MultiGetRequirementResponse, { id: number }>;
@Component({
  selector: 'app-requirements',
  imports: [
    RouterLink,
    ReactiveFormsModule,
    MatButtonModule,
    MatCardModule,
    MatFormFieldModule,
    MatInputModule,
    MatSelectModule,
  ],
  templateUrl: './requirements.html',
  styleUrl: './requirements.scss',
})
export class Requirements {
  private readonly api = inject(RequirementsApi);
  private readonly fb = inject(FormBuilder).nonNullable;
  readonly loading = signal(true);
  readonly busy = signal(false);
  readonly error = signal('');
  readonly message = signal('');
  readonly repository = signal<MultiGetRepositoryResponse | undefined>(undefined);
  readonly items = signal<MultiListRequirementsResponse['requirements']>([]);
  readonly selected = signal<Requirement | undefined>(undefined);
  readonly reviewing = signal(false);
  readonly selectedRepository = signal(1);
  readonly registering = signal(false);
  // Retain a key for an identical retry, including a lost HTTP response.
  private pending = { fingerprint: '', key: '' };
  readonly form = this.fb.group({
    title: ['', Validators.required],
    description: ['', Validators.required],
    acceptance_criteria: this.fb.array([this.criterion()]),
    validation_plan: this.fb.array([this.step()]),
    network_access: [''],
  });
  readonly setup = this.fb.group({
    project: ['', Validators.required],
    remote: ['', Validators.required],
    github_repository_id: [1, [Validators.required, Validators.min(1)]],
    base_branch: ['main', Validators.required],
    token_limit: [100000, Validators.min(1)],
    turn_limit: [20, Validators.min(1)],
    model_work_seconds: [3600, Validators.min(1)],
    gate_recovery_policy: ['bounded_v1', Validators.required],
  });
  constructor() {
    void this.reload();
  }
  criterion() {
    return this.fb.group({
      description: ['', Validators.required],
      verification_ref: ['test', Validators.required],
    });
  }
  step() {
    return this.fb.group({
      id: ['test', Validators.required],
      check: ['cargo_test', Validators.required],
      selector: ['', Validators.required],
      expected_result: ['', Validators.required],
      timeout_seconds: [60, [Validators.required, Validators.min(1), Validators.max(3600)]],
    });
  }
  addCriterion() {
    this.form.controls.acceptance_criteria.push(this.criterion());
  }
  addStep() {
    this.form.controls.validation_plan.push(this.step());
  }
  selectRepository(id: number) {
    this.selectedRepository.set(id);
    this.form.markAsDirty();
    this.reviewing.set(false);
  }
  async reload() {
    this.loading.set(true);
    await this.run(async () => {
      const [repository, list] = await Promise.all([
        firstValueFrom(this.api.repository()),
        firstValueFrom(this.api.list()),
      ]);
      this.repository.set(repository);
      this.items.set(list.requirements);
    });
    this.loading.set(false);
  }
  async run(action: () => Promise<void>) {
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    this.message.set('');
    try {
      await action();
    } catch (error) {
      if (error instanceof HttpErrorResponse && error.status === 409 && this.selected()) {
        try {
          this.accept(await firstValueFrom(this.api.detail(this.selected()!.id)));
        } catch {
          /* Keep last visible facts; show failure below. */
        }
      }
      this.error.set(
        '操作未完成。请检查表单与仓库授权；版本冲突时重新打开需求。网络失败可重试原操作。',
      );
    } finally {
      this.busy.set(false);
    }
  }
  key(payload: unknown) {
    const fingerprint = JSON.stringify(payload);
    this.pending =
      this.pending.fingerprint === fingerprint
        ? this.pending
        : { fingerprint, key: crypto.randomUUID() };
    return this.pending.key;
  }
  async configure() {
    if (this.setup.invalid) {
      this.setup.markAllAsTouched();
    } else {
      await this.run(async () => {
        const value = this.setup.getRawValue();
        const payload = {
          version: 0,
          repository_id:
            Math.max(0, ...(this.repository()?.repositories.map((entry) => entry.id ?? 1) ?? [])) +
            1,
          repository: {
            project: value.project,
            remote: value.remote,
            github_repository_id: value.github_repository_id,
            base_branch: value.base_branch,
            revoked: false,
            reason: 'Local user reviewed initial repository policy',
            policy: {
              allowed_checks: ['cargo_test', 'npm_test'],
              max_timeout_seconds: 3600,
              token_limit: value.token_limit,
              turn_limit: value.turn_limit,
              model_work_seconds: value.model_work_seconds,
              gate_recovery_policy: value.gate_recovery_policy,
            },
          },
        };
        await firstValueFrom(this.api.configure({ ...payload, request_id: this.key(payload) }));
        this.repository.set(await firstValueFrom(this.api.repository()));
        this.selectedRepository.set(payload.repository_id);
        this.registering.set(false);
        this.message.set('仓库策略已保存，交付能力尚未检查。');
      });
    }
  }
  async open(id: number) {
    await this.run(async () => {
      this.accept(await firstValueFrom(this.api.detail(id)));
    });
  }
  accept(response: MultiGetRequirementResponse) {
    if (!('id' in response)) {
      throw new Error(response.error);
    } else {
      this.selected.set(response);
      this.selectedRepository.set(response.repository_id ?? 1);
      this.reviewing.set(false);
      this.form.controls.acceptance_criteria.clear();
      this.form.controls.validation_plan.clear();
      for (const ac of response.contract.acceptance_criteria) {
        const group = this.criterion();
        group.setValue(ac);
        this.form.controls.acceptance_criteria.push(group);
      }
      for (const step of response.contract.validation_plan) {
        const group = this.step();
        group.setValue(step);
        this.form.controls.validation_plan.push(group);
      }
      this.form.patchValue({
        title: response.contract.title,
        description: response.contract.description,
        network_access: response.contract.network_access.join(', '),
      });
      this.form.markAsPristine();
    }
  }
  newDraft() {
    this.selected.set(undefined);
    this.reviewing.set(false);
    this.form.reset();
    this.form.controls.acceptance_criteria.clear();
    this.form.controls.validation_plan.clear();
    this.addCriterion();
    this.addStep();
  }
  async save() {
    if (this.form.invalid) {
      this.form.markAllAsTouched();
    } else {
      await this.run(async () => {
        const raw = this.form.getRawValue();
        const contract: MultiCreateRequirementRequest['contract'] = {
          ...raw,
          network_access: raw.network_access
            .split(',')
            .map((s) => s.trim())
            .filter(Boolean),
        };
        const selected = this.selected();
        const payload = {
          version: selected?.version ?? 0,
          repository_id: this.selectedRepository(),
          contract,
        };
        const body = { ...payload, request_id: this.key({ id: selected?.id, ...payload }) };
        const response = selected
          ? await firstValueFrom(this.api.update(selected.id, body))
          : await firstValueFrom(this.api.create(body));
        this.accept(response);
        this.items.set((await firstValueFrom(this.api.list())).requirements);
        this.message.set('Draft 已保存，可以评审。');
      });
    }
  }
  async control(action: 'ready' | 'withdraw') {
    const selected = this.selected();
    if (!selected) {
      return;
    } else {
      await this.run(async () => {
        const payload = {
          version: selected.version,
          repository_version:
            this.repository()?.repositories.find(
              (entry) => (entry.id ?? 1) === this.selectedRepository(),
            )?.version ?? 0,
          request_id: this.key({ id: selected.id, version: selected.version, action }),
        };
        const response =
          action === 'ready'
            ? await firstValueFrom(this.api.ready(selected.id, payload))
            : await firstValueFrom(this.api.withdraw(selected.id, payload));
        this.accept(response);
        this.items.set((await firstValueFrom(this.api.list())).requirements);
        this.message.set(
          action === 'ready'
            ? 'Ready 已持久化，等待运行与仓库就绪能力。关闭浏览器不会删除队列记录。'
            : '已撤回 Draft，历史快照仍保留。',
        );
      });
    }
  }
}
