import { HttpClient } from '@angular/common/http';
import { inject, Service } from '@angular/core';
import {
  AnswerOperatorQuestionRequest,
  AnswerOperatorQuestionResponse,
  ControlOperationsRequest,
  ControlOperationsResponse,
  GetEvidenceResponse,
  GetInboxResponse,
  GetOperationsResponse,
} from './health-response';

@Service()
export class OperationsApi {
  private readonly http = inject(HttpClient);
  detail(id: number) {
    return this.http.get<GetOperationsResponse>(`/api/requirements/${id}/operations`);
  }
  inbox() {
    return this.http.get<GetInboxResponse>('/api/inbox');
  }
  control(id: number, body: ControlOperationsRequest) {
    return this.http.post<ControlOperationsResponse>(
      `/api/requirements/${id}/operations`,
      body,
    );
  }
  answer(id: string, body: AnswerOperatorQuestionRequest) {
    return this.http.post<AnswerOperatorQuestionResponse>(
      `/api/operator/questions/${id}/answer`,
      body,
    );
  }
  evidence(id: number, run: string, channel: string) {
    return this.http.get<GetEvidenceResponse>(`/api/requirements/${id}/evidence/${run}/${channel}`);
  }
}
