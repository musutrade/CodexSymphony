-- Intent precedes filesystem mutation; complete references follow verification.
-- Ordering never depends on wall-clock movement or lexicographic Run IDs.
ALTER TABLE agent_run ADD COLUMN run_sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE;
CREATE TABLE run_workspace (
 run_id text PRIMARY KEY REFERENCES agent_run(id),
 identity jsonb NOT NULL,
 candidate_sha text,
 restored_from text REFERENCES agent_run(id)
);
CREATE TABLE workspace_operation (
 run_id text NOT NULL REFERENCES agent_run(id),
 request_id text NOT NULL,
 command jsonb NOT NULL,
 status text NOT NULL CHECK (status IN ('pending','partial','complete')),
 result jsonb,
 error text,
 PRIMARY KEY(run_id,request_id),
 CHECK ((status='complete') = (result IS NOT NULL))
);
CREATE UNIQUE INDEX one_unfinished_workspace_operation ON workspace_operation(run_id)
 WHERE status != 'complete';
CREATE TABLE workspace_snapshot (
 run_id text PRIMARY KEY REFERENCES agent_run(id),
 manifest jsonb NOT NULL,
 candidate boolean NOT NULL
);
