-- Explicit original-item recovery. This is not validation failure evidence.
CREATE TABLE pre_merge_recovery (
 id text PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 merge_key text NOT NULL UNIQUE REFERENCES merge_operation(action_key),
 request jsonb NOT NULL,
 state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','ready','blocked')),
 failure_id text UNIQUE REFERENCES linked_failure(id),
 attempts integer NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 3),
 next_attempt_at bigint NOT NULL DEFAULT 0,
 blocker text,
 receipts jsonb NOT NULL DEFAULT '[]',
 created_at timestamptz NOT NULL DEFAULT now()
);
