import { HttpClient } from '@angular/common/http';
import { Service, inject } from '@angular/core';
import {
  EditGroupQueueRequest,
  EditGroupQueueResponse,
  GetGroupReviewResponse,
  SaveGroupReviewRequest,
  SaveGroupReviewResponse,
  AuthorizeGroupRequest,
  AuthorizeGroupResponse,
} from './health-response';
@Service()
export class GroupReviewApi {
  private readonly http = inject(HttpClient);
  private readonly options = { headers: { 'x-codexsymphony-csrf': '1' } };
  edit(id: string, body: EditGroupQueueRequest) {
    return this.http.post<EditGroupQueueResponse>(
      `/api/drafts/${id}/queue-edit`,
      body,
      this.options,
    );
  }
  read(id: string) {
    return this.http.get<GetGroupReviewResponse>(`/api/drafts/${id}/review`);
  }
  save(id: string, body: SaveGroupReviewRequest) {
    return this.http.put<SaveGroupReviewResponse>(`/api/drafts/${id}/review`, body, this.options);
  }
  confirm(id: string, body: AuthorizeGroupRequest) {
    return this.http.post<AuthorizeGroupResponse>(
      `/api/drafts/${id}/authorize`,
      body,
      this.options,
    );
  }
}
