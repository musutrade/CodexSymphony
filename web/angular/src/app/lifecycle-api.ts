import { HttpClient } from '@angular/common/http';
import { inject, Service } from '@angular/core';
import {
  GetExtensionRecoveryResponse,
  GetLifecycleHistoryResponse,
  GetRequirementModelsResponse,
  ReplayNotificationRequest,
  ReplayNotificationResponse,
  ResolveExtensionRecoveryRequest,
  ResolveExtensionRecoveryResponse,
} from './health-response';

@Service()
export class LifecycleApi {
  private readonly http = inject(HttpClient);
  recovery(id: number) {
    return this.http.get<GetExtensionRecoveryResponse>(
      `/api/requirements/${id}/extension-recovery`,
    );
  }
  resolve(id: number, body: ResolveExtensionRecoveryRequest) {
    return this.http.post<ResolveExtensionRecoveryResponse>(
      `/api/requirements/${id}/extension-recovery`,
      body,
    );
  }
  events(id: number, after: number) {
    return this.http.get<GetLifecycleHistoryResponse>(
      `/api/requirements/${id}/lifecycle`,
      { params: { after } },
    );
  }
  replay(id: number, body: ReplayNotificationRequest) {
    return this.http.post<ReplayNotificationResponse>(
      `/api/requirements/${id}/notifications/replay`,
      body,
    );
  }
  models(id: number) {
    return this.http.get<GetRequirementModelsResponse>(`/api/requirements/${id}/models`);
  }
}
