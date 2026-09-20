-- Review/edit versions and immutable authorizations are separate from execution.
CREATE TABLE group_review (
 draft_id text PRIMARY KEY REFERENCES imported_draft(id),
 version bigint NOT NULL CHECK(version > 0),
 draft_revision bigint NOT NULL,
 document jsonb NOT NULL,
 FOREIGN KEY(draft_id,draft_revision) REFERENCES imported_draft_revision(draft_id,version)
);
CREATE TABLE group_review_revision (
 draft_id text NOT NULL REFERENCES imported_draft(id),
 version bigint NOT NULL,
 draft_revision bigint NOT NULL,
 document jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(draft_id,version),
 FOREIGN KEY(draft_id,draft_revision) REFERENCES imported_draft_revision(draft_id,version)
);
CREATE TABLE group_authorization (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 draft_id text NOT NULL REFERENCES imported_draft(id),
 review_version bigint NOT NULL,
 request_id text NOT NULL UNIQUE,
 input jsonb NOT NULL,
 snapshot jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(draft_id,review_version),
 FOREIGN KEY(draft_id,review_version) REFERENCES group_review_revision(draft_id,version)
);
CREATE TABLE group_queue (
 draft_id text PRIMARY KEY REFERENCES imported_draft(id),
 authorization_id bigint NOT NULL UNIQUE REFERENCES group_authorization(id),
 state text NOT NULL CHECK(state IN ('waiting_scheduler','needs_review')),
 version bigint NOT NULL DEFAULT 1,
 created_at timestamptz NOT NULL DEFAULT now()
);
-- Stable draft/child identities, independent of revision or current membership.
-- Empty item_id is the parent-group ledger; it never owns an execution slot.
CREATE TABLE group_budget (
 draft_id text NOT NULL REFERENCES imported_draft(id),
 item_id text NOT NULL,
 limits jsonb NOT NULL,
 used jsonb NOT NULL DEFAULT '{"tokens":0,"turns":0,"model_seconds":0}',
 reserved jsonb NOT NULL DEFAULT '{"tokens":0,"turns":0,"model_seconds":0}',
 PRIMARY KEY(draft_id,item_id)
);
