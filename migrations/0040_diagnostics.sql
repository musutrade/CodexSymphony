-- Content and manifest become visible in one transaction. Neither raw payload
-- nor producer filesystem paths are available through product read endpoints.
CREATE TABLE diagnostic_artifact (
 sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
 artifact_id text PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 source_run text NOT NULL,
 invocation_id text NOT NULL,
 attempt integer NOT NULL CHECK(attempt > 0),
 manifest jsonb NOT NULL,
 raw_payload bytea,
 export_payload bytea,
 allocated_bytes bigint NOT NULL CHECK(allocated_bytes >= 0),
 expires_at bigint NOT NULL,
 retired_at bigint,
 UNIQUE(invocation_id,attempt,artifact_id),
 CHECK((raw_payload IS NULL) = (export_payload IS NULL))
);
CREATE INDEX diagnostic_task_page ON diagnostic_artifact(requirement_id,sequence);
CREATE INDEX diagnostic_source ON diagnostic_artifact(source_run,invocation_id);
REVOKE ALL ON diagnostic_artifact FROM PUBLIC;

-- Retained evidence keeps the producer revision's repository authority. A newer
-- authorized task revision must not reopen a revoked historical repository.
CREATE FUNCTION diagnostic_revision_allows(task bigint, rev bigint)
RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT EXISTS(
  SELECT 1 FROM execution_revision v
  JOIN repository p ON p.id=plugin_scope_repository(task,rev)
  WHERE v.requirement_id=task AND v.revision=rev
    AND NOT COALESCE((p.document->>'revoked')::boolean,false)
    AND p.revoked_through_version<COALESCE((v.document->>'repository_version')::bigint,p.version)
 )
$$;

-- An unrelated Run of the same Requirement is not a repair authorization.
CREATE FUNCTION diagnostic_run_allows(reader text, source text, invocation text)
RETURNS boolean LANGUAGE plpgsql STABLE AS $$
BEGIN
 RETURN reader=source OR EXISTS(
  SELECT 1 FROM repair_reservation p
  LEFT JOIN candidate_validation v ON v.id=p.source_validation_id
  LEFT JOIN linked_failure f ON f.id=p.linked_failure_id
  WHERE p.repair_run_id=reader AND p.status='started'
    AND (v.id=invocation OR f.source_run=source)
 ) OR EXISTS(
  SELECT 1 FROM runtime_session s WHERE s.run_id=reader AND s.source_run=source
 );
END;
$$;
