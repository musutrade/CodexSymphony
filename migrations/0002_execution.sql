ALTER TABLE requirement ADD COLUMN paused boolean NOT NULL DEFAULT false;

-- A singleton survives process restarts and spans all execution/delivery phases.
CREATE TABLE execution_control (
 id integer PRIMARY KEY CHECK (id = 1),
 requirement_id bigint UNIQUE REFERENCES requirement(id),
 paused boolean NOT NULL DEFAULT false,
 incarnation text,
 recovery_complete boolean NOT NULL DEFAULT false
);
INSERT INTO execution_control(id) VALUES (1);

CREATE TABLE agent_run (
 id text PRIMARY KEY,
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 incarnation text NOT NULL,
 request_id text NOT NULL,
 workspace text NOT NULL,
 workspace_identity text NOT NULL,
 launch jsonb NOT NULL,
 state text NOT NULL CHECK (state IN ('Created','Running','Succeeded','Failed','Interrupted')),
 phase text NOT NULL DEFAULT 'execution',
 process_identity jsonb,
 stop_requested boolean NOT NULL DEFAULT false,
 quiescent boolean NOT NULL DEFAULT false,
 blocker text,
 created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY (requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision),
 UNIQUE(id,request_id,incarnation)
);
CREATE UNIQUE INDEX one_active_run_per_requirement ON agent_run(requirement_id)
 WHERE state IN ('Created','Running');

CREATE TABLE run_event (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 run_id text NOT NULL,
 request_id text NOT NULL,
 incarnation text NOT NULL,
 payload jsonb NOT NULL,
 accepted boolean NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
