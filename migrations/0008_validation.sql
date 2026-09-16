-- Identity-bound validation facts are separate from AgentRun execution facts.
CREATE TABLE candidate_validation (
  id text PRIMARY KEY,
  requirement_id bigint NOT NULL REFERENCES requirement(id),
  revision bigint NOT NULL,
  source_run_id text NOT NULL REFERENCES agent_run(id),
  candidate_sha text NOT NULL,
  candidate_tree text NOT NULL,
  trusted jsonb NOT NULL,
  source_before text NOT NULL,
  source_after text NOT NULL,
  entry_before text NOT NULL,
  entry_after text NOT NULL,
  stage text NOT NULL CHECK (stage IN ('declaration','validation','repair_reservation','handoff','done')),
  result text NOT NULL CHECK (result IN ('pending','succeeded','gate_failed','blocked')),
  repair_count integer NOT NULL DEFAULT 0 CHECK (repair_count BETWEEN 0 AND 1),
  UNIQUE(requirement_id, revision, candidate_sha)
);
CREATE TABLE validation_step (
  validation_id text NOT NULL REFERENCES candidate_validation(id),
  step_id text NOT NULL,
  command jsonb NOT NULL,
  exit_code integer,
  output text,
  output_sha256 text,
  log_ref text,
  consumer text,
  code_failure boolean NOT NULL DEFAULT false,
  status text NOT NULL CHECK (status IN ('pending','succeeded','failed','unknown')),
  PRIMARY KEY(validation_id, step_id)
);
CREATE TABLE repair_reservation (
  requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
  ordinal bigint NOT NULL UNIQUE,
  source_validation_id text NOT NULL REFERENCES candidate_validation(id),
  repair_run_id text UNIQUE REFERENCES agent_run(id),
  failure jsonb NOT NULL,
  status text NOT NULL CHECK (status IN ('reserved','started','succeeded','failed'))
);
