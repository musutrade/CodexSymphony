import { Routes } from '@angular/router';
import { HealthPage } from './health-page';
export const routes: Routes = [
  { path: '', component: HealthPage },
  { path: 'drafts', loadComponent: () => import('./drafts').then((m) => m.Drafts) },
  { path: 'inbox', loadComponent: () => import('./operations').then((m) => m.Operations) },
  {
    path: 'requirements/list',
    loadComponent: () => import('./requirement-list').then((m) => m.RequirementList),
  },
  {
    path: 'requirements/:id',
    loadComponent: () => import('./operations').then((m) => m.Operations),
  },
  {
    path: 'requirements',
    loadComponent: () => import('./requirements/requirements').then((m) => m.Requirements),
  },
];
