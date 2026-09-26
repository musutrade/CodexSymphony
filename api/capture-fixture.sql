-- HTTP-only data fixtures; no real model, credentials or GitHub writes.
-- IDs are outside the normal scenario sequence; global scheduler stays paused.
-- HTTP listening is liveness, not completion of the asynchronous recovery barrier.
-- Observe the real barrier before replaying commands that legitimately reject
-- recovery-in-progress. Never manufacture recovery or storage readiness here.
DO $$
BEGIN
  FOR attempt IN 1..100 LOOP
    IF (SELECT recovery_complete FROM execution_control WHERE id=1)
       AND NOT (SELECT blocked FROM storage_guard WHERE id=1) THEN
      RETURN;
    END IF;
    PERFORM pg_sleep(0.1);
  END LOOP;
  RAISE EXCEPTION 'HTTP fixture requires completed recovery and available storage';
END;
$$;

INSERT INTO requirement(id,version,state,contract,revision,paused)
OVERRIDING SYSTEM VALUE VALUES(900001,1,'Running','{"title": "Persisted HTTP fixture", "description": "Implement a future assertion", "acceptance_criteria": [{"description": "Future test passes", "verification_ref": "test"}], "validation_plan": [{"id": "test", "check": "cargo_test", "selector": "future::passes", "expected_result": "exit 0 and assertions pass", "timeout_seconds": 60}], "network_access": []}',1,true);
INSERT INTO requirement_revision(requirement_id,revision,document)
VALUES(900001,1,'{"repository_version":1}');
INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent)
VALUES('capture-run',900001,1,'http-fixture','capture-request','fixture','fixture','{}','Interrupted',true);
INSERT INTO runtime_session(run_id,created_at,last_progress) VALUES('capture-run',1,1);
INSERT INTO runtime_question(id,requirement_id,revision,run_id,rpc_id,original,created_at)
VALUES('capture-question',900001,1,'capture-run','1','{"params":{"questions":[{"id":"choice","question":"Which option?","options":[{"label":"yes"}]}]}}',1);
INSERT INTO runtime_evidence(run_id,channel,kept_bytes,truncated) VALUES('capture-run','stdout',21,false);
INSERT INTO runtime_evidence_chunk(run_id,channel,sequence,payload)
VALUES('capture-run','stdout',1,convert_to(E'Fixture-owned output\n','UTF8'));

-- Synthetic terminal generation is a replay fixture, never AC01 model evidence.
INSERT INTO draft_generation(id,request,fingerprint,draft_id,input_version,status,usage,limits,error,completed_at) VALUES('capture-generation','{"request_id": "capture-generation", "draft_id": null, "version": 0, "label": "contract fixture", "text": "synthetic generation input"}','fixture-generation-900001','draft-capture-generation',0,'failed','{"input":null,"cached":null,"output":null,"model_seconds":null,"complete":false}','{"tokens":30000,"turns":1,"model_seconds":120}','synthetic terminal failure',now());

-- Historical decision replay fixture only; real acceptance/execution is covered
-- by extension_lifecycle, not inferred from this retained HTTP response.
INSERT INTO business_request(request_id,input,result) VALUES('capture-extension-recovery','{"extension_recovery": 900001, "command": {"request_id": "capture-extension-recovery", "version": 1, "revision": 1, "validation_id": "capture-validation", "reason": "Retained HTTP recovery replay", "action": {"kind": "revalidate", "plan_digest": "1111111111111111111111111111111111111111111111111111111111111111", "resume_condition": "Reviewed implementation available"}}}','{"accepted": true, "started": false, "version": 2, "event_key": "capture-extension:validation", "resolution_state": "pending"}');
