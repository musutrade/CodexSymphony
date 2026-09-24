-- Hook intent is durable before a script starts. Unknown effects are retained.
CREATE TABLE project_hook_run (
 run_id text PRIMARY KEY,
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 resource_id text NOT NULL,
 workspace text NOT NULL,
 role text NOT NULL,
 frozen jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE project_hook_invocation (
 invocation_id text PRIMARY KEY,
 run_id text NOT NULL REFERENCES project_hook_run(run_id),
 resource_id text NOT NULL,
 event text NOT NULL,
 hook_name text NOT NULL,
 attempt integer NOT NULL DEFAULT 1 CHECK (attempt BETWEEN 1 AND 2),
 status text NOT NULL CHECK (status IN ('intent','running','success','failed','timeout','cancelled','unknown')),
 pid integer,
 process_identity jsonb,
 stop_confirmed boolean NOT NULL DEFAULT false,
 output_dir text NOT NULL,
 result jsonb,
 diagnostic text,
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(run_id,resource_id,event,hook_name)
);
