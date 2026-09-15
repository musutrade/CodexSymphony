import { Component } from '@angular/core';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
@Component({
  selector: 'app-root',
  imports: [RouterLink, RouterLinkActive, RouterOutlet],
  templateUrl: './app.html',
})
export class App {
  readonly navigation = [
    { path: '/requirements', label: '需求工作台' },
    { path: '/', label: '服务状态' },
  ];
}
