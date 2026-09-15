import { Service } from '@angular/core';
import { httpResource } from '@angular/common/http';
import { HealthResponse } from './health-response';

@Service()
export class Health {
  readonly response = httpResource<HealthResponse>(() => '/api/health');
}
