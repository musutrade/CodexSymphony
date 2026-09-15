import { HttpClient } from '@angular/common/http';
import { inject, Service } from '@angular/core';
import {
  ConfigureRepositoryRequest,
  ConfigureRepositoryResponse,
  CreateRequirementRequest,
  CreateRequirementResponse,
  GetRepositoryResponse,
  GetRequirementResponse,
  ListRequirementsResponse,
  ReadyRequirementRequest,
  ReadyRequirementResponse,
  UpdateRequirementRequest,
  UpdateRequirementResponse,
  WithdrawRequirementRequest,
  WithdrawRequirementResponse,
} from '../health-response';

@Service()
export class RequirementsApi {
  private readonly http = inject(HttpClient);
  private readonly options = { headers: { 'x-codexsymphony-csrf': '1' } };
  repository() {
    return this.http.get<GetRepositoryResponse>('/api/repository');
  }
  list() {
    return this.http.get<ListRequirementsResponse>('/api/requirements');
  }
  detail(id: number) {
    return this.http.get<GetRequirementResponse>(`/api/requirements/${id}`);
  }
  configure(body: ConfigureRepositoryRequest) {
    return this.http.put<ConfigureRepositoryResponse>('/api/repository', body, this.options);
  }
  create(body: CreateRequirementRequest) {
    return this.http.post<CreateRequirementResponse>('/api/requirements', body, this.options);
  }
  update(id: number, body: UpdateRequirementRequest) {
    return this.http.patch<UpdateRequirementResponse>(
      `/api/requirements/${id}`,
      body,
      this.options,
    );
  }
  ready(id: number, body: ReadyRequirementRequest) {
    return this.http.post<ReadyRequirementResponse>(
      `/api/requirements/${id}/ready`,
      body,
      this.options,
    );
  }
  withdraw(id: number, body: WithdrawRequirementRequest) {
    return this.http.post<WithdrawRequirementResponse>(
      `/api/requirements/${id}/withdraw`,
      body,
      this.options,
    );
  }
}
