import { Routes } from '@angular/router';
import { HealthPage } from './health-page';
export const routes: Routes = [
  { path: '', component: HealthPage },
  {
    path: 'requirements',
    loadComponent: () => import('./requirements/requirements').then((m) => m.Requirements),
  },
];
