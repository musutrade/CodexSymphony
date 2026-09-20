import { HttpClient } from '@angular/common/http';
import { inject, Service } from '@angular/core';
import {
  MultiConfigureRepositoryRequest,
  MultiConfigureRepositoryResponse,
  MultiCreateRequirementRequest,
  MultiCreateRequirementResponse,
  MultiGetRepositoryResponse,
  MultiGetRequirementResponse,
  MultiListRequirementsResponse,
  MultiReadyRequirementRequest,
  MultiReadyRequirementResponse,
  MultiUpdateRequirementRequest,
  MultiUpdateRequirementResponse,
  MultiWithdrawRequirementRequest,
  MultiWithdrawRequirementResponse,
} from '../health-response';

@Service()
export class RequirementsApi {
  private readonly http = inject(HttpClient);
  private readonly options = { headers: { 'x-codexsymphony-csrf': '1' } };
  repository() {
    return this.http.get<MultiGetRepositoryResponse>('/api/multi/repository');
  }
  list() {
    return this.http.get<MultiListRequirementsResponse>('/api/multi/requirements');
  }
  detail(id: number) {
    return this.http.get<MultiGetRequirementResponse>(`/api/multi/requirements/${id}`);
  }
  configure(body: MultiConfigureRepositoryRequest) {
    return this.http.put<MultiConfigureRepositoryResponse>(
      '/api/multi/repository',
      body,
      this.options,
    );
  }
  create(body: MultiCreateRequirementRequest) {
    return this.http.post<MultiCreateRequirementResponse>(
      '/api/multi/requirements',
      body,
      this.options,
    );
  }
  update(id: number, body: MultiUpdateRequirementRequest) {
    return this.http.patch<MultiUpdateRequirementResponse>(
      `/api/multi/requirements/${id}`,
      body,
      this.options,
    );
  }
  ready(id: number, body: MultiReadyRequirementRequest) {
    return this.http.post<MultiReadyRequirementResponse>(
      `/api/multi/requirements/${id}/ready`,
      body,
      this.options,
    );
  }
  withdraw(id: number, body: MultiWithdrawRequirementRequest) {
    return this.http.post<MultiWithdrawRequirementResponse>(
      `/api/multi/requirements/${id}/withdraw`,
      body,
      this.options,
    );
  }
}
