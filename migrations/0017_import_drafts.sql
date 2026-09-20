-- Draft groups have a separate, non-executable namespace. Existing requirements,
-- revisions, budgets and the global execution owner are intentionally untouched.
CREATE TABLE imported_draft (
 id text PRIMARY KEY CHECK (id LIKE 'draft-%'),
 version bigint NOT NULL CHECK (version > 0),
 document jsonb NOT NULL,
 source jsonb NOT NULL,
 source_sha256 text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE imported_draft_revision (
 draft_id text NOT NULL REFERENCES imported_draft(id),
 version bigint NOT NULL,
 document jsonb NOT NULL,
 source jsonb NOT NULL,
 source_sha256 text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(draft_id, version)
);
