-- Existing authorizations keep the historical limit. New authorization explicitly
-- opts in through the reviewed repository policy; migration grants nothing.
CREATE TABLE repair_authorization (
 requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
 repair_limit bigint NOT NULL CHECK (repair_limit > 0),
 policy text NOT NULL
);
INSERT INTO repair_authorization SELECT DISTINCT requirement_id,1,'one_code_repair' FROM requirement_revision;
ALTER TABLE repair_reservation DROP CONSTRAINT repair_reservation_pkey;
ALTER TABLE repair_reservation DROP CONSTRAINT repair_reservation_ordinal_check;
ALTER TABLE repair_reservation ADD PRIMARY KEY(requirement_id,ordinal);
ALTER TABLE repair_reservation ADD CHECK(ordinal > 0);
CREATE UNIQUE INDEX repair_failure_once ON repair_reservation(source_validation_id);
CREATE UNIQUE INDEX repair_one_pending ON repair_reservation(requirement_id) WHERE status IN ('reserved','started');
ALTER TABLE repair_reservation ADD COLUMN resources jsonb;
ALTER TABLE repair_reservation ADD COLUMN resources_transferred boolean NOT NULL DEFAULT false;
ALTER TABLE repair_reservation ADD COLUMN event_key text UNIQUE;
CREATE TABLE recovery_failure (
 event_key text PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 source_validation_id text NOT NULL REFERENCES candidate_validation(id),
 phase text NOT NULL,
 facts jsonb NOT NULL,
 fingerprint text NOT NULL,
 decision text NOT NULL,
 reason text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE recovery_retry (
 event_key text PRIMARY KEY REFERENCES recovery_failure(event_key),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 2),
 deadline bigint NOT NULL,
 next_attempt_at bigint NOT NULL,
 state text NOT NULL CHECK(state IN ('pending','unknown','complete','blocked')),
 remote_attempt bigint,
 remote jsonb,
 probe_failures integer NOT NULL DEFAULT 0,
 receipts jsonb NOT NULL DEFAULT '[]'
);
ALTER TABLE candidate_validation ADD COLUMN retry_of text REFERENCES candidate_validation(id);
ALTER TABLE candidate_validation DROP CONSTRAINT candidate_validation_requirement_id_revision_candidate_sha_key;
CREATE UNIQUE INDEX validation_initial_candidate ON candidate_validation(requirement_id,revision,candidate_sha) WHERE retry_of IS NULL;
CREATE TABLE repair_intent_history (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 requirement_id bigint NOT NULL,
 ordinal bigint NOT NULL,
 launch jsonb NOT NULL,
 workspace jsonb NOT NULL,
 reason text NOT NULL
);
ALTER TABLE delivery DROP CONSTRAINT delivery_repository_id_branch_key;
ALTER TABLE delivery ADD COLUMN original_action_key text REFERENCES delivery(action_key);
ALTER TABLE delivery ADD COLUMN expected_head text;
CREATE UNIQUE INDEX delivery_candidate_branch ON delivery(repository_id,branch,head_sha);
ALTER TABLE delivery ADD COLUMN superseded_by text REFERENCES delivery(action_key);
-- A resumed Runtime Run continues the same repair; late callbacks still belong
-- to their original Run and can never become a second code-repair allocation.
CREATE TABLE repair_run_history (
 requirement_id bigint NOT NULL,
 ordinal bigint NOT NULL,
 source_run text PRIMARY KEY REFERENCES agent_run(id),
 successor_run text UNIQUE NOT NULL REFERENCES agent_run(id),
 FOREIGN KEY(requirement_id,ordinal) REFERENCES repair_reservation(requirement_id,ordinal)
);
