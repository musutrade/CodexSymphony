-- No historical authorization is upgraded. Scope must explicitly opt in.
CREATE TABLE linked_failure (
 id text PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 merge_key text REFERENCES merge_operation(action_key),
 integration_id text REFERENCES integration_validation(id),
 evidence jsonb NOT NULL,
 required_steps jsonb NOT NULL,
 state text NOT NULL DEFAULT 'observed' CHECK(state IN ('observed','reserved','merged','complete','blocked','cancelled')),
 blocker text,
 repository_id bigint REFERENCES repository(id),
 document jsonb,
 paths jsonb,
 baseline text,
 manifest jsonb,
 source_run text REFERENCES agent_run(id),
 repair_delivery text REFERENCES delivery(action_key),
 final_version jsonb,
 created_at timestamptz NOT NULL DEFAULT now(),
 CHECK ((merge_key IS NULL) <> (integration_id IS NULL))
);
CREATE UNIQUE INDEX linked_merge_failure ON linked_failure(merge_key) WHERE merge_key IS NOT NULL;
CREATE UNIQUE INDEX linked_integration_failure ON linked_failure(integration_id) WHERE integration_id IS NOT NULL;
ALTER TABLE repair_reservation ALTER COLUMN source_validation_id DROP NOT NULL;
ALTER TABLE repair_reservation ADD COLUMN linked_failure_id text UNIQUE REFERENCES linked_failure(id);
ALTER TABLE repair_reservation ADD CHECK ((source_validation_id IS NULL) <> (linked_failure_id IS NULL));
-- Run input remains immutable even after a later repair switches repository.
CREATE TABLE linked_run_input (
 run_id text PRIMARY KEY REFERENCES agent_run(id),
 failure_id text NOT NULL REFERENCES linked_failure(id),
 document jsonb NOT NULL
);
-- Current execution input only. The reviewed revision and child kind never change.
-- Keep the bounded lookup out of callers' large join graphs. PL/pgSQL prevents
-- view inlining from multiplying the coordinator's authorization query planner.
CREATE FUNCTION linked_execution_document(wanted_requirement bigint, wanted_revision bigint, original jsonb)
RETURNS jsonb LANGUAGE plpgsql STABLE AS $$
DECLARE overridden jsonb;
BEGIN
 SELECT document INTO overridden FROM linked_failure
 WHERE requirement_id=wanted_requirement AND revision=wanted_revision
   AND state IN ('observed','reserved') AND baseline IS NOT NULL
 ORDER BY created_at DESC,id DESC LIMIT 1;
 RETURN COALESCE(overridden,original);
END;
$$;
CREATE INDEX linked_current_input ON linked_failure(requirement_id,revision,created_at DESC,id DESC)
 WHERE state IN ('observed','reserved') AND baseline IS NOT NULL;
CREATE VIEW execution_revision AS
 SELECT requirement_id,revision,linked_execution_document(requirement_id,revision,document) AS document
 FROM requirement_revision;
ALTER TABLE linked_failure ADD COLUMN revalidation_id text REFERENCES integration_validation(id);
ALTER TABLE linked_failure ADD COLUMN source_attempts integer NOT NULL DEFAULT 0;
ALTER TABLE linked_failure ADD COLUMN next_source_at bigint NOT NULL DEFAULT 0;

ALTER TABLE linked_failure ADD COLUMN source_receipts jsonb NOT NULL DEFAULT '[]';
CREATE INDEX linked_requirement_history ON linked_failure(requirement_id,created_at DESC);
CREATE UNIQUE INDEX linked_delivery_once ON linked_failure(repair_delivery) WHERE repair_delivery IS NOT NULL;
