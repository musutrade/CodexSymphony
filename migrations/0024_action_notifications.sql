-- Committed actionable facts only. The notifier role receives SELECT on this
-- projection, never access to answers, contracts, credentials or business writes.
CREATE VIEW notification_action AS
WITH current_requirement AS (
 SELECT * FROM requirement WHERE NOT cancel_requested
), actions AS (
 SELECT id AS requirement_id, revision, 'cancel_cleanup'::text AS kind,
   jsonb_build_array(revision) AS identity
 FROM requirement WHERE cancel_requested AND NOT cleanup_complete
 UNION ALL
 SELECT id,revision,'paused',jsonb_build_array(revision)
 FROM current_requirement WHERE paused
 UNION ALL
 SELECT r.id,r.revision,'failed',jsonb_build_array(r.revision,
   (SELECT a.id FROM agent_run a WHERE a.requirement_id=r.id AND a.revision=r.revision
    ORDER BY a.created_at DESC,a.id DESC LIMIT 1))
 FROM current_requirement r WHERE state='Failed'
 UNION ALL
 SELECT r.id,r.revision,'question',jsonb_build_array(q.id,q.version,q.resume_state)
 FROM current_requirement r JOIN runtime_question q ON q.requirement_id=r.id AND q.revision=r.revision
 WHERE q.resume_state IN ('waiting','pending')
 UNION ALL
 SELECT r.id,r.revision,'run_blocker',jsonb_build_array(a.id,a.blocker)
 FROM current_requirement r JOIN LATERAL (
   SELECT id,blocker FROM agent_run WHERE requirement_id=r.id AND revision=r.revision
   ORDER BY created_at DESC,id DESC LIMIT 1
 ) a ON a.blocker IS NOT NULL
 UNION ALL
 SELECT r.id,r.revision,'preparation',jsonb_build_array(p.run_id,p.retry#>>'{last_failure,code}',p.retry->'attempts')
 FROM current_requirement r JOIN preparation_record p ON p.requirement_id=r.id AND p.revision=r.revision
 WHERE NOT p.ready AND (p.retry->>'todo')::boolean
 UNION ALL
 SELECT r.id,r.revision,'delivery',jsonb_build_array(d.action_key,a.kind,a.error->>'code',a.attempt_limit)
 FROM current_requirement r JOIN delivery d ON d.requirement_id=r.id
 JOIN delivery_action a USING(action_key) WHERE NOT d.released AND a.state='blocked'
 UNION ALL
 SELECT r.id,r.revision,'validation',jsonb_build_array(v.id,v.stage,v.failure)
 FROM current_requirement r JOIN candidate_validation v ON v.requirement_id=r.id AND v.revision=r.revision
 WHERE v.result='blocked' AND v.stage<>'done'
 UNION ALL
 SELECT DISTINCT a.requirement_id,r.revision,'storage_guard',
   jsonb_build_array(g.blocked,g.error,(g.scan_retry->>'todo')::boolean,
                     COALESCE((g.measured->>'classification_todo')::bigint,0)>0)
 FROM storage_attempt a JOIN requirement r ON r.id=a.requirement_id CROSS JOIN storage_guard g
 WHERE g.blocked OR (g.scan_retry->>'todo')::boolean OR (g.measured->>'classification_todo')::bigint>0
 UNION ALL
 SELECT a.requirement_id,a.revision,'storage_material',
   jsonb_build_array(m.id,m.protection,m.retry#>>'{last_failure,code}')
 FROM storage_material m JOIN storage_attempt a USING(run_id)
 WHERE (m.retry->>'todo')::boolean OR m.protection IN
 ('partial archive; reconcile','unknown identity','unknown preparation identity','partial/pending requires reconciliation')
 UNION ALL
 SELECT a.requirement_id,a.revision,'evidence_cleanup',
   jsonb_build_array(e.run_id,e.channel,e.cleanup_retry#>>'{last_failure,code}')
 FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id WHERE (e.cleanup_retry->>'todo')::boolean
)
SELECT requirement_id,kind,
 -- No free-form source text leaves the database projection.
 encode(sha256(convert_to(jsonb_build_array(requirement_id,revision,kind,identity,
   (SELECT max(e.version) FROM business_event e
    WHERE e.object_id='requirement:' || actions.requirement_id::text AND e.kind=CASE actions.kind
      WHEN 'paused' THEN 'operator_pause'
      WHEN 'preparation' THEN 'operator_recheck'
      WHEN 'run_blocker' THEN 'operator_recheck'
      WHEN 'storage_guard' THEN 'operator_storage_recheck'
      WHEN 'storage_material' THEN 'operator_storage_recheck'
      WHEN 'evidence_cleanup' THEN 'operator_storage_recheck'
      WHEN 'delivery' THEN 'operator_delivery_recheck' END))::text,'UTF8')),'hex') AS action_key
FROM actions;
