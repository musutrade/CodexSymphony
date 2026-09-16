-- Protocol transport lifetime is shorter than business question lifetime.
CREATE TABLE runtime_session (
 run_id text PRIMARY KEY REFERENCES agent_run(id),
 thread_id text UNIQUE,
 turn_id text,
 call_id text,
 connected boolean NOT NULL DEFAULT false,
 end_kind text CHECK(end_kind IN ('completion','blocker')),
 end_request jsonb,
 end_payload jsonb,
 source_run text REFERENCES agent_run(id),
 created_at bigint NOT NULL,
 waiting_since bigint,
 last_progress bigint NOT NULL
);
CREATE TABLE runtime_request (
 run_id text NOT NULL REFERENCES runtime_session(run_id),
 rpc_id jsonb NOT NULL,
 original jsonb NOT NULL CHECK(pg_column_size(original)<=65536),
 result jsonb CHECK(pg_column_size(result)<=65536),
 PRIMARY KEY(run_id,rpc_id)
);
CREATE TABLE runtime_question (
 id text PRIMARY KEY,
 version bigint NOT NULL DEFAULT 1,
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 run_id text NOT NULL REFERENCES runtime_session(run_id),
 rpc_id jsonb NOT NULL,
 original jsonb NOT NULL CHECK(pg_column_size(original)<=65536),
 created_at bigint NOT NULL,
 answer jsonb CHECK(pg_column_size(answer)<=65536),
 answered_at bigint,
 resume_state text NOT NULL DEFAULT 'waiting' CHECK(resume_state IN ('waiting','pending','live','linked','invalid')),
 resumed_run text REFERENCES agent_run(id),
 FOREIGN KEY(requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision),
 UNIQUE(run_id,rpc_id)
);
CREATE TABLE runtime_blocker (
 run_id text NOT NULL REFERENCES agent_run(id),
 code text NOT NULL,
 detail jsonb NOT NULL CHECK(pg_column_size(detail)<=65536),
 resolved boolean NOT NULL DEFAULT false,
 PRIMARY KEY(run_id,code)
);
-- One bounded stream per channel and Run/attempt. Byte admission/retention across
-- Runs belongs to GH-23; model tokens are never a disk budget.
CREATE TABLE runtime_evidence (
 run_id text NOT NULL REFERENCES agent_run(id),
 channel text NOT NULL CHECK(channel IN ('stdout','stderr','attachment')),
 kept_bytes bigint NOT NULL DEFAULT 0,
 discarded_bytes bigint NOT NULL DEFAULT 0,
 records integer NOT NULL DEFAULT 0,
 truncated boolean NOT NULL DEFAULT false,
 PRIMARY KEY(run_id,channel)
);
CREATE TABLE runtime_evidence_chunk (
 run_id text NOT NULL,
 channel text NOT NULL,
 sequence integer NOT NULL,
 payload bytea NOT NULL CHECK(octet_length(payload)<=65536),
 PRIMARY KEY(run_id,channel,sequence),
 FOREIGN KEY(run_id,channel) REFERENCES runtime_evidence(run_id,channel)
);
-- Restoration intent survives controller failure; unknown filesystem work is
-- retained for reconciliation instead of creating another worktree each tick.
CREATE TABLE runtime_resume (
 source_run text PRIMARY KEY REFERENCES agent_run(id),
 job jsonb NOT NULL,
 status text NOT NULL CHECK(status IN ('restoring','prepared','dispatched'))
);
