import { HttpClient } from '@angular/common/http';
import { inject, Service } from '@angular/core';
import {
  GetDraftResponse,
  ImportDraftRequest,
  ImportDraftResponse,
  ListDraftsResponse,
  UpdateDraftRequest,
  UpdateDraftResponse,
} from './health-response';
@Service()
export class DraftsApi {
  private readonly http = inject(HttpClient);
  list() {
    return this.http.get<ListDraftsResponse>('/api/drafts');
  }
  read(id: string) {
    return this.http.get<GetDraftResponse>(`/api/drafts/${id}`);
  }
  create(body: ImportDraftRequest) {
    return this.http.post<ImportDraftResponse>('/api/drafts', body);
  }
  update(id: string, body: UpdateDraftRequest) {
    return this.http.put<UpdateDraftResponse>(`/api/drafts/${id}`, body);
  }
}
