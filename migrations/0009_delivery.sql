ALTER TABLE requirement DROP CONSTRAINT requirement_state_check;
ALTER TABLE requirement ADD CONSTRAINT requirement_state_check
 CHECK (state IN ('Draft','Ready','Running','Submitted','Failed','Cancelled'));
ALTER TABLE requirement ADD COLUMN cancel_requested boolean NOT NULL DEFAULT false;
ALTER TABLE requirement ADD COLUMN cleanup_complete boolean NOT NULL DEFAULT false;

-- Immutable validated artifact and delivery identity. No evidence compaction is
-- performed here: consumers remain explicit until a later retention policy acts.
CREATE TABLE delivery (
 action_key text PRIMARY KEY,
 validation_id text NOT NULL UNIQUE REFERENCES candidate_validation(id),
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 repository_id bigint NOT NULL,
 repository text NOT NULL,
 branch text NOT NULL,
 base_branch text NOT NULL,
 head_sha text NOT NULL,
 manifest jsonb NOT NULL,
 policy jsonb NOT NULL,
 pr_number bigint,
 consumer text NOT NULL DEFAULT 'outbox',
 released boolean NOT NULL DEFAULT false,
 UNIQUE(repository_id,branch),
 FOREIGN KEY(requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision)
);
CREATE TABLE delivery_action (
 action_key text NOT NULL REFERENCES delivery(action_key),
 kind text NOT NULL CHECK(kind IN ('publish','close')),
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','unknown','confirmed','blocked','withdrawn')),
 attempts integer NOT NULL DEFAULT 0,
 next_attempt_at bigint NOT NULL DEFAULT 0,
 error jsonb,
 PRIMARY KEY(action_key,kind)
);
CREATE TABLE delivery_attempt (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 action_key text NOT NULL,
 kind text NOT NULL,
 ordinal integer NOT NULL,
 operation text NOT NULL,
 result jsonb,
 created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(action_key,kind) REFERENCES delivery_action(action_key,kind),
 UNIQUE(action_key,kind,ordinal)
);
CREATE TABLE delivery_observation (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 action_key text NOT NULL REFERENCES delivery(action_key),
 kind text NOT NULL,
 fact jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);

-- Initial Ready-to-Run preparation is saved before creating a worktree. It uses
-- the existing preparation ledger and never allocates a second Run on retry.
CREATE TABLE initial_run (
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 launch jsonb NOT NULL,
 workspace jsonb NOT NULL,
 PRIMARY KEY(requirement_id,revision),
 FOREIGN KEY(requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision)
);

-- Distinguish an explicitly paused coding Run from an unexplained failure.
ALTER TABLE agent_run ADD COLUMN user_paused boolean NOT NULL DEFAULT false;
