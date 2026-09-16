-- One account per executable Requirement; all its immutable revisions and Runs
-- share this key. Re-review never overwrites its first authorization.
CREATE TABLE requirement_budget (
 requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
 limits jsonb NOT NULL,
 version bigint NOT NULL DEFAULT 1,
 exhausted boolean NOT NULL DEFAULT false
);
CREATE TABLE budget_authorization (
 requirement_id bigint NOT NULL REFERENCES requirement_budget(requirement_id),
 version bigint NOT NULL,
 request_id text NOT NULL UNIQUE,
 actor text NOT NULL,
 reason text NOT NULL,
 delta jsonb NOT NULL,
 limits jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(requirement_id,version)
);
-- Preserve the earliest already-reviewed authorization when upgrading a fixture.
INSERT INTO requirement_budget(requirement_id,limits)
 SELECT DISTINCT ON (requirement_id) requirement_id,
 jsonb_build_object('tokens',document#>'{repository,policy,token_limit}',
 'turns',document#>'{repository,policy,turn_limit}',
 'model_seconds',document#>'{repository,policy,model_work_seconds}')
 FROM requirement_revision WHERE document#>'{repository,policy,token_limit}' IS NOT NULL
 ORDER BY requirement_id,revision;
INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits)
 SELECT requirement_id,1,'initial-budget:'||requirement_id,'local-user','initial reviewed authorization',limits,limits FROM requirement_budget;
ALTER TABLE agent_run ADD COLUMN model text;
UPDATE agent_run a SET model=v.document#>>'{repository,model}' FROM requirement_revision v
 WHERE v.requirement_id=a.requirement_id AND v.revision=a.revision;
ALTER TABLE agent_run ADD COLUMN waiting jsonb NOT NULL DEFAULT '{"human_seconds":0,"paused_seconds":0,"ci_seconds":0,"network_seconds":0}';
CREATE TABLE model_call (
 run_id text NOT NULL REFERENCES agent_run(id),
 turn_id text NOT NULL,
 requirement_id bigint NOT NULL REFERENCES requirement_budget(requirement_id),
 intent jsonb NOT NULL,
 reserved jsonb NOT NULL,
 usage jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(run_id,turn_id)
);
CREATE TABLE model_usage_event (
 run_id text NOT NULL,
 turn_id text NOT NULL,
 event_id text NOT NULL,
 usage jsonb NOT NULL,
 received_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(run_id,turn_id,event_id),
 FOREIGN KEY(run_id,turn_id) REFERENCES model_call(run_id,turn_id)
);
