-- Opt-in authorization only; existing approved groups remain unchanged.
CREATE TABLE integration_validation (
 id text PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 authorization_id bigint NOT NULL REFERENCES group_authorization(id),
 revision bigint NOT NULL,
 binding jsonb NOT NULL,
 job jsonb NOT NULL,
 launch jsonb NOT NULL,
 state text NOT NULL CHECK(state IN ('prepared','executing','unknown','passed','failed','cancelled','interrupted')),
 process_identity jsonb,
 quiescent boolean NOT NULL DEFAULT false,
 result jsonb,
 blocker text,
 next_attempt_at bigint,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE group_acceptance (
 draft_id text NOT NULL REFERENCES imported_draft(id),
 authorization_id bigint NOT NULL REFERENCES group_authorization(id),
 draft_revision bigint NOT NULL,
 review_version bigint NOT NULL,
 validation_id text NOT NULL REFERENCES integration_validation(id),
 evidence jsonb NOT NULL,
 PRIMARY KEY(draft_id,authorization_id,draft_revision,review_version),
 created_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX integration_one_live ON integration_validation(requirement_id) WHERE NOT quiescent;
