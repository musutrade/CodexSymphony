-- HTTP-only data fixtures; no real model, credentials or GitHub writes.
-- IDs are outside the normal scenario sequence; global scheduler stays paused.
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
