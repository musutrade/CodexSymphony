-- Durable committed transitions, not a periodically sampled current-state view.
CREATE TABLE lifecycle_event (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 sequence bigint NOT NULL,
 phase text NOT NULL,
 source_id text NOT NULL,
 facts jsonb NOT NULL,
 occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(requirement_id,sequence)
);
CREATE TABLE notification_plugin (
 id text PRIMARY KEY,
 database_role name NOT NULL UNIQUE,
 enabled boolean NOT NULL DEFAULT false
);
CREATE TABLE notification_delivery (
 plugin_id text NOT NULL REFERENCES notification_plugin(id),
 event_id bigint NOT NULL REFERENCES lifecycle_event(id),
 state text NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','unknown','accepted','ignored','failed')),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 6),
 attempt_limit integer NOT NULL DEFAULT 3 CHECK(attempt_limit IN (3,6)),
 next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 deadline timestamptz NOT NULL DEFAULT clock_timestamp()+interval '10 minutes',
 last_result text,
 PRIMARY KEY(plugin_id,event_id)
);

CREATE TABLE notification_attempt (
 plugin_id text NOT NULL,
 event_id bigint NOT NULL,
 ordinal integer NOT NULL,
 started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 result text NOT NULL DEFAULT 'unknown',
 PRIMARY KEY(plugin_id,event_id,ordinal),
 FOREIGN KEY(plugin_id,event_id) REFERENCES notification_delivery(plugin_id,event_id)
);

CREATE FUNCTION lifecycle_append(task bigint, rev bigint, phase_name text, source text, data jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE event_id bigint; ordinal bigint;
BEGIN
 -- Serialize numbering with the same requirement row. Rollback removes both
 -- business change and event/outbox rows; sequences may have harmless gaps.
 PERFORM 1 FROM requirement WHERE id=task FOR UPDATE;
 IF NOT FOUND THEN RETURN; END IF;
 SELECT COALESCE(max(sequence),0)+1 INTO ordinal FROM lifecycle_event WHERE requirement_id=task;
 INSERT INTO lifecycle_event(requirement_id,revision,sequence,phase,source_id,facts)
 VALUES(task,rev,ordinal,phase_name,source,data) RETURNING id INTO event_id;
 INSERT INTO notification_delivery(plugin_id,event_id)
 SELECT id,event_id FROM notification_plugin WHERE enabled;
END $$;

CREATE FUNCTION lifecycle_capture() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE current_row jsonb:=to_jsonb(NEW); old_row jsonb:='{}';
 task bigint; rev bigint; source text; selected jsonb:='{}'; previous jsonb:='{}'; field text;
BEGIN
 IF TG_OP='UPDATE' THEN old_row:=to_jsonb(OLD); END IF;
 FOREACH field IN ARRAY string_to_array(TG_ARGV[1],',') LOOP
  selected:=selected||jsonb_build_object(field,current_row->field);
  previous:=previous||jsonb_build_object(field,old_row->field);
 END LOOP;
 IF TG_TABLE_NAME='preparation_record' THEN
  selected:=selected||jsonb_build_object('attempts',current_row#>'{retry,attempts}','todo',current_row#>'{retry,todo}');
  previous:=previous||jsonb_build_object('attempts',old_row#>'{retry,attempts}','todo',old_row#>'{retry,todo}');
 END IF;
 IF TG_OP='UPDATE' AND selected=previous THEN RETURN NEW; END IF;
 source:=COALESCE(current_row->>'id',current_row->>'invocation_id',current_row->>'event_key',current_row->>'action_key',current_row->>'run_id',current_row->>'source_run');
 IF TG_TABLE_NAME='requirement' THEN
  task:=NEW.id; rev:=NEW.revision;
 ELSIF TG_TABLE_NAME='delivery_action' THEN
  SELECT requirement_id,revision INTO task,rev FROM delivery WHERE action_key=NEW.action_key;
 ELSIF TG_TABLE_NAME='runtime_resume' THEN
  SELECT requirement_id,revision INTO task,rev FROM agent_run WHERE id=NEW.source_run;
 ELSIF TG_TABLE_NAME='recovery_retry' THEN
  SELECT f.requirement_id,v.revision INTO task,rev FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id WHERE f.event_key=NEW.event_key;
 ELSIF TG_TABLE_NAME IN ('workspace_operation','workspace_snapshot','run_workspace','runtime_blocker') THEN
  SELECT requirement_id,revision INTO task,rev FROM agent_run WHERE id=NEW.run_id;
 ELSIF TG_TABLE_NAME='project_hook_invocation' THEN
  SELECT requirement_id,revision INTO task,rev FROM project_hook_run WHERE run_id=NEW.run_id;
 ELSE
  task:=(current_row->>'requirement_id')::bigint;
  SELECT revision INTO rev FROM requirement WHERE id=task;
  rev:=COALESCE((current_row->>'revision')::bigint,rev);
 END IF;
 IF task IS NOT NULL THEN
  PERFORM lifecycle_append(task,rev,TG_ARGV[0],COALESCE(source,task::text),
    jsonb_build_object('status',selected,'previous',previous,'transition',lower(TG_OP)));
 END IF;
 RETURN NEW;
END $$;

-- Only explicitly selected structural facts leave the database. No contracts,
-- answers, free-form logs, arbitrary plugin output, paths or credentials.
CREATE TRIGGER lifecycle_requirement AFTER INSERT OR UPDATE ON requirement FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('requirement','state,revision,paused,cancel_requested,cleanup_complete');
CREATE TRIGGER lifecycle_preparation AFTER INSERT OR UPDATE ON preparation_record FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('preparation','ready');
CREATE TRIGGER lifecycle_execution AFTER INSERT OR UPDATE ON agent_run FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('execution','state,phase,quiescent,stop_requested');
CREATE TRIGGER lifecycle_question AFTER INSERT OR UPDATE ON runtime_question FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('question','version,resume_state');
CREATE TRIGGER lifecycle_validation AFTER INSERT OR UPDATE ON candidate_validation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('validation','stage,result,retry_of,hook_invalidated,superseded_by');
CREATE TRIGGER lifecycle_delivery AFTER INSERT OR UPDATE ON delivery FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('delivery','released,superseded_by');
CREATE TRIGGER lifecycle_delivery_action AFTER INSERT OR UPDATE ON delivery_action FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('delivery_action','kind,state,attempts');
CREATE TRIGGER lifecycle_merge AFTER INSERT OR UPDATE ON merge_operation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('merge','state,pre_validation_started,merge_started,acceptance_started');
CREATE TRIGGER lifecycle_integration AFTER INSERT OR UPDATE ON integration_validation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('integration','state,quiescent');
CREATE TRIGGER lifecycle_recovery AFTER INSERT OR UPDATE ON recovery_failure FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('recovery','decision,resolution_state,successor_validation');
CREATE TRIGGER lifecycle_hook AFTER INSERT OR UPDATE ON project_hook_invocation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('hook','event,status,attempt,stop_confirmed');

CREATE TRIGGER lifecycle_workspace AFTER INSERT OR UPDATE ON run_workspace FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('workspace','candidate_sha');
CREATE TRIGGER lifecycle_preservation AFTER INSERT OR UPDATE ON workspace_snapshot FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('preservation','candidate');
CREATE TRIGGER lifecycle_workspace_operation AFTER INSERT OR UPDATE ON workspace_operation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('workspace_operation','status');

CREATE TRIGGER lifecycle_blocker AFTER INSERT OR UPDATE ON runtime_blocker FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('blocker','code,resolved');
CREATE TRIGGER lifecycle_resume AFTER INSERT OR UPDATE ON runtime_resume FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('resume','status');
CREATE TRIGGER lifecycle_retry AFTER INSERT OR UPDATE ON recovery_retry FOR EACH ROW
 EXECUTE FUNCTION lifecycle_capture('retry','state,attempts');

CREATE FUNCTION lifecycle_environment_capture() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE selected jsonb; previous jsonb;
BEGIN
 PERFORM 1 FROM requirement WHERE id=NEW.requirement_id FOR UPDATE;
 selected:=jsonb_build_object('stage',NEW.stage,'passed',COALESCE(NEW.report->'error'='null'::jsonb AND NEW.report->'differences'='[]'::jsonb AND NEW.report->'response'<>'null'::jsonb,false),'actual_digest',NEW.report->'actual_digest');
 SELECT facts->'status' INTO previous FROM lifecycle_event
 WHERE requirement_id=NEW.requirement_id AND revision=NEW.revision AND phase='environment' AND facts#>>'{status,stage}'=NEW.stage ORDER BY sequence DESC LIMIT 1;
 IF selected IS NOT DISTINCT FROM previous THEN RETURN NEW; END IF;
 PERFORM lifecycle_append(NEW.requirement_id,NEW.revision,'environment',NEW.id::text,
 jsonb_build_object('status',selected,'previous',COALESCE(previous,'{}'::jsonb),'transition','observation'));
 RETURN NEW;
END $$;
CREATE TRIGGER lifecycle_environment AFTER INSERT ON environment_observation FOR EACH ROW
 EXECUTE FUNCTION lifecycle_environment_capture();

-- The notifier login can execute only these functions. Resolve tables against
-- the installation schema, never an untrusted caller's search_path.
CREATE FUNCTION notification_claim(plugin text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $$
DECLARE delivery notification_delivery%ROWTYPE; event lifecycle_event%ROWTYPE;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM notification_plugin WHERE id=plugin AND database_role=session_user AND enabled) THEN
  RAISE EXCEPTION 'notification plugin not authorized';
 END IF;
 UPDATE notification_delivery SET state='failed',last_result='retry_exhausted'
 WHERE plugin_id=plugin AND state IN ('pending','unknown') AND
 (deadline<=clock_timestamp() OR (attempts>=attempt_limit AND next_attempt_at<=clock_timestamp()));
 SELECT * INTO delivery FROM notification_delivery d WHERE plugin_id=plugin
 AND state IN ('pending','unknown') AND attempts<attempt_limit AND next_attempt_at<=clock_timestamp()
 AND deadline>clock_timestamp()+interval '15 seconds'
 AND NOT EXISTS(SELECT 1 FROM notification_delivery earlier JOIN lifecycle_event a ON a.id=earlier.event_id
 JOIN lifecycle_event b ON b.id=d.event_id WHERE earlier.plugin_id=plugin AND earlier.state IN ('pending','unknown')
 AND a.requirement_id=b.requirement_id AND a.sequence<b.sequence)
 ORDER BY event_id FOR UPDATE SKIP LOCKED LIMIT 1;
 IF NOT FOUND THEN RETURN NULL; END IF;
 UPDATE notification_delivery SET state='unknown',attempts=attempts+1,
 next_attempt_at=clock_timestamp()+CASE WHEN attempts=0 THEN interval '30 seconds' ELSE interval '120 seconds' END,
 last_result='unconfirmed' WHERE plugin_id=plugin AND event_id=delivery.event_id;
 INSERT INTO notification_attempt(plugin_id,event_id,ordinal) VALUES(plugin,delivery.event_id,delivery.attempts+1);
 SELECT * INTO event FROM lifecycle_event WHERE id=delivery.event_id;
 RETURN jsonb_build_object('protocol_version',1,'plugin_id',plugin,'event_id',event.id,
 'attempt',delivery.attempts+1,'requirement_id',event.requirement_id,'revision',event.revision,
 'sequence',event.sequence,'phase',event.phase,'source_id',event.source_id,'facts',event.facts,
 'occurred_at',event.occurred_at,'detail_path','/requirements/'||event.requirement_id);
END $$;

CREATE FUNCTION notification_ack(plugin text, event bigint, attempt integer, result text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $$
DECLARE changed integer;
BEGIN
 IF NOT EXISTS(SELECT 1 FROM notification_plugin WHERE id=plugin AND database_role=session_user AND enabled) THEN
  RAISE EXCEPTION 'notification plugin not authorized';
 END IF;
 IF result NOT IN ('accepted','ignored','failed','unknown') THEN RAISE EXCEPTION 'invalid notification acknowledgement'; END IF;
 UPDATE notification_delivery SET state=result,last_result=result WHERE plugin_id=plugin AND event_id=event
 AND attempts=attempt AND state='unknown';
 GET DIAGNOSTICS changed=ROW_COUNT;
 IF changed=1 THEN UPDATE notification_attempt SET result=notification_ack.result WHERE plugin_id=plugin AND event_id=event AND ordinal=attempt; END IF;
 RETURN changed=1;
END $$;
REVOKE ALL ON FUNCTION notification_claim(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION notification_ack(text,bigint,integer,text) FROM PUBLIC;
REVOKE ALL ON notification_plugin,notification_delivery,notification_attempt,lifecycle_event FROM PUBLIC;
