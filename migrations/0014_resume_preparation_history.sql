-- Keep undispatched prepared recovery work when a new controller must re-preflight.
CREATE TABLE runtime_resume_history (
 run_id text PRIMARY KEY,
 source_run text NOT NULL REFERENCES agent_run(id),
 job jsonb NOT NULL,
 status text NOT NULL CHECK(status='prepared'),
 reason text NOT NULL,
 retired_at timestamptz NOT NULL DEFAULT now()
);
