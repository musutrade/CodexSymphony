-- Integration checks do not create AgentRuns. Grant their original reports
-- only to the repair reservation linked to that exact failed invocation.
CREATE FUNCTION diagnostic_integration_allows(task bigint, rev bigint, source text, invocation text, phase text)
RETURNS boolean LANGUAGE plpgsql STABLE AS $$
DECLARE versions jsonb;
BEGIN
 IF phase IS DISTINCT FROM 'integration' AND COALESCE(phase,'') NOT LIKE 'integration/step:%'
    AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE id=source OR id=invocation) THEN
  RETURN true;
 END IF;
 SELECT binding->'versions' INTO versions FROM integration_validation
 WHERE id=source AND id=invocation AND requirement_id=task AND revision=rev;
 IF jsonb_typeof(versions) IS DISTINCT FROM 'array' THEN RETURN false; END IF;
 RETURN jsonb_array_length(versions)>0 AND NOT EXISTS(
  SELECT 1 FROM jsonb_array_elements(versions) bound(item)
  LEFT JOIN repository p ON p.id=(bound.item->>'repository_id')::bigint
  WHERE p.id IS NULL OR COALESCE((p.document->>'revoked')::boolean,false)
     OR bound.item->>'repository_version' IS NULL
     OR p.version<(bound.item->>'repository_version')::bigint
     OR p.revoked_through_version>=(bound.item->>'repository_version')::bigint
 );
END;
$$;

CREATE OR REPLACE FUNCTION diagnostic_run_allows(reader text, source text, invocation text)
RETURNS boolean LANGUAGE plpgsql STABLE AS $$
BEGIN
 RETURN reader=source OR EXISTS(
  SELECT 1 FROM repair_reservation p
  LEFT JOIN candidate_validation v ON v.id=p.source_validation_id
  LEFT JOIN linked_failure f ON f.id=p.linked_failure_id
  WHERE p.repair_run_id=reader AND p.status='started'
    AND (v.id=invocation OR f.source_run=source
         OR (f.integration_id=source AND f.integration_id=invocation
             AND diagnostic_integration_allows(f.requirement_id,f.revision,source,invocation,'integration')))
 ) OR EXISTS(
  SELECT 1 FROM runtime_session s WHERE s.run_id=reader AND s.source_run=source
 );
END;
$$;
