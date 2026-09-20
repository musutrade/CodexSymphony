-- A generation is an advisory model call, never an executable AgentRun.
CREATE TABLE draft_generation (
 id text PRIMARY KEY,
 request jsonb NOT NULL,
 fingerprint text NOT NULL UNIQUE,
 draft_id text NOT NULL,
 input_version bigint NOT NULL CHECK(input_version >= 0),
 output_version bigint,
 status text NOT NULL CHECK(status IN ('running','succeeded','failed','interrupted','conflict')),
 error text,
 output text,
 evidence jsonb NOT NULL DEFAULT '{}',
 usage jsonb NOT NULL,
 limits jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 completed_at timestamptz
);
CREATE UNIQUE INDEX one_draft_generation_running ON draft_generation ((true)) WHERE status='running';
