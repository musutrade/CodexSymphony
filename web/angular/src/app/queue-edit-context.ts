import { InjectionToken, Signal } from '@angular/core';
import { GroupView } from './group-review-model';
/** The queue editor shares the parent's persisted view and reload transaction. */
export interface QueueEditContext {
  view: Signal<GroupView | undefined>;
  reload(): Promise<void>;
}
export const QUEUE_EDIT_CONTEXT = new InjectionToken<QueueEditContext>('Group queue review');
