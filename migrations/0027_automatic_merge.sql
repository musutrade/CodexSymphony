-- Additive, opt-in ledger. Unknown sends retain the original owner indefinitely.
ALTER TABLE candidate_validation ADD COLUMN approved_plan jsonb;
ALTER TABLE requirement DROP CONSTRAINT requirement_state_check;
ALTER TABLE requirement ADD CONSTRAINT requirement_state_check
 CHECK (state IN ('Draft','Ready','Running','Submitted','Done','Failed','Cancelled'));
CREATE TABLE merge_operation (
 action_key text PRIMARY KEY,
 delivery_key text NOT NULL REFERENCES delivery(action_key),
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 intent jsonb NOT NULL,
 state text NOT NULL CHECK(state IN ('prepared','unknown','merged','complete','blocked','invalidated','cancelled')),
 created_at bigint NOT NULL,
 next_attempt_at bigint NOT NULL,
 merged_sha text,
 merged_at bigint,
 merge_method text,
 receipts jsonb NOT NULL DEFAULT '[]',
 dispatches jsonb NOT NULL DEFAULT '[]',
 acceptance jsonb,
 acceptance_started boolean NOT NULL DEFAULT false,
 merge_started boolean NOT NULL DEFAULT false,
 pre_validation_started boolean NOT NULL DEFAULT false,
 pre_validation jsonb,
 blocker text,
 UNIQUE(delivery_key,action_key)
);
CREATE UNIQUE INDEX merge_operation_active ON merge_operation(delivery_key)
 WHERE state<>'invalidated';
