import { MatButtonModule } from '@angular/material/button';
import { Auth } from './auth';
import { Login } from './login/login';
import { Component, inject, signal } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
@Component({
  selector: 'app-root',
  imports: [MatButtonModule, Login, RouterLink, RouterLinkActive, RouterOutlet],
  templateUrl: './app.html',
})
export class App {
  readonly auth = inject(Auth);
  readonly logoutError = signal('');
  async logout() {
    this.logoutError.set('');
    try {
      await this.auth.logout();
    } catch {
      this.logoutError.set('退出失败，请检查连接并重试。');
    }
  }
  readonly navigation = [
    { path: '/requirements', label: '需求工作台' },
    { path: '/requirements/list', label: '需求列表' },
    { path: '/inbox', label: '待办箱' },
    { path: '/', label: '服务状态' },
  ];
}
