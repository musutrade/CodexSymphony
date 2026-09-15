import { Component, inject } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { Health } from './health';

@Component({
  selector: 'app-health-page',
  imports: [MatButtonModule, MatCardModule],
  templateUrl: './health-page.html',
  styleUrl: './app.scss',
})
export class HealthPage {
  protected readonly health = inject(Health);
}
