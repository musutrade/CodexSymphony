import { Component, inject, signal, afterNextRender } from '@angular/core';
import { FormField, form, required, maxLength } from '@angular/forms/signals';
import { ActivatedRoute, Router } from '@angular/router';
import { HttpErrorResponse } from '@angular/common/http';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { Auth, safeReturn } from '../auth';
@Component({
  selector: 'app-login',
  imports: [FormField, MatButtonModule, MatFormFieldModule, MatInputModule],
  template: ` <section class="login-card" aria-labelledby="login-title">
    <h1 id="login-title" tabindex="-1">登录 CodexSymphony</h1>
    @if (continuation) {
      <p role="status">
        会话已结束，请重新登录。未提交输入保留在当前页面；登录后请检查并手动提交。
      </p>
    }
    <form (submit)="submit($event)">
      <mat-form-field
        ><mat-label>用户名</mat-label>
        <input matInput [formField]="fields.username" autocomplete="username" />
      </mat-form-field>
      <mat-form-field
        ><mat-label>密码</mat-label>
        <input
          matInput
          [formField]="fields.password"
          type="password"
          autocomplete="current-password"
        />
      </mat-form-field>
      @if (error()) {
        <p id="login-error" role="alert">{{ error() }}</p>
      }
      <button mat-flat-button type="submit" [disabled]="busy()">
        {{ busy() ? '正在登录…' : '登录' }}
      </button>
    </form>
    <p>账号由宿主管理员初始化。忘记密码时请联系管理员重置。</p>
  </section>`,
  styles: `
    :host {
      display: block;
    }
    .login-card {
      max-width: 28rem;
      margin: var(--ui-space-6, 24px) auto;
      padding: 24px;
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: 12px;
      background: var(--mat-sys-surface);
    }
    form {
      display: grid;
      gap: 16px;
    }
    mat-form-field {
      width: 100%;
    }
    button {
      min-height: 44px;
    }
    [role='alert'] {
      color: var(--mat-sys-error);
    }
    @media (max-width: 480px) {
      .login-card {
        padding: 16px;
        margin: 0;
      }
    }
  `,
})
export class Login {
  private readonly auth = inject(Auth);
  readonly continuation = this.auth.expired();
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);
  readonly model = signal({ username: '', password: '' });
  readonly fields = form(this.model, (fields) => {
    required(fields.username);
    maxLength(fields.username, 128);
    required(fields.password);
    maxLength(fields.password, 1024);
  });
  readonly busy = signal(false);
  readonly error = signal('');
  constructor() {
    afterNextRender(() => document.getElementById('login-title')?.focus());
  }
  async submit(event: Event) {
    event.preventDefault();
    if (this.busy()) return;
    this.busy.set(true);
    this.error.set('');
    try {
      const value = this.model();
      await this.auth.login(value.username, value.password);
      this.model.update((value) => {
        return { ...value, password: '' };
      });
      if (!this.continuation)
        await this.router.navigateByUrl(
          safeReturn(this.route.snapshot.queryParamMap.get('return')),
        );
    } catch (error: unknown) {
      this.error.set(
        error instanceof HttpErrorResponse && error.status === 429
          ? '用户名或密码错误；尝试次数过多，请 15 分钟后重试。'
          : '用户名或密码错误。若服务不可用，请检查连接后重试。',
      );
    } finally {
      this.busy.set(false);
    }
  }
}
