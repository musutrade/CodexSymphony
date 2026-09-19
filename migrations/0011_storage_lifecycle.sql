-- Product storage facts. These tables never own Requirement/Run/Gate outcomes.
CREATE TABLE storage_policy (
 version text PRIMARY KEY,
 document jsonb NOT NULL,
 deployment jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE storage_guard ADD COLUMN policy_version text REFERENCES storage_policy(version);
ALTER TABLE storage_guard ADD COLUMN measured jsonb;
ALTER TABLE storage_guard ADD COLUMN measured_at bigint;
ALTER TABLE storage_guard ADD COLUMN scan_retry jsonb;

-- Preparation attempts exist before agent_run; do not require a final report.
CREATE TABLE storage_attempt (
 run_id text PRIMARY KEY,
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 identity jsonb NOT NULL,
 sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
 resolved_by text REFERENCES storage_attempt(run_id),
 reclaim_proof jsonb,
 created_at bigint NOT NULL DEFAULT extract(epoch FROM now())::bigint,
 expires_at bigint NOT NULL,
 summary jsonb NOT NULL DEFAULT '{}',
 FOREIGN KEY(requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision)
);
CREATE TABLE storage_material (
 id text PRIMARY KEY,
 run_id text NOT NULL REFERENCES storage_attempt(run_id),
 path text NOT NULL UNIQUE,
 directory_identity jsonb NOT NULL,
 kind text NOT NULL CHECK(kind IN ('decision','retrospective','recovery','rebuildable')),
 category text NOT NULL CHECK(category IN ('workspace','hot','cold','database','record')),
 allocation_request text NOT NULL,
 policy_version text NOT NULL REFERENCES storage_policy(version),
 created_at bigint NOT NULL,
 expires_at bigint NOT NULL,
 actual_bytes bigint NOT NULL DEFAULT 0 CHECK(actual_bytes>=0),
 manifest jsonb,
 archive jsonb,
 protection text,
 status text NOT NULL DEFAULT 'available' CHECK(status IN ('available','archiving','archived','deleting','deleted')),
 retry jsonb NOT NULL,
 deleted_at bigint,
 deletion_reason text
);
CREATE TABLE storage_allocation (
 request_id text PRIMARY KEY,
 run_id text NOT NULL REFERENCES storage_attempt(run_id),
 category text NOT NULL CHECK(category IN ('workspace','hot','cold','database','record')),
 policy_version text NOT NULL REFERENCES storage_policy(version),
 requested bigint NOT NULL CHECK(requested>0),
 allocated bigint NOT NULL CHECK(allocated>0),
 outstanding bigint NOT NULL CHECK(outstanding>=0),
 actual bigint NOT NULL DEFAULT 0 CHECK(actual>=0),
 settled boolean NOT NULL DEFAULT false,
 CHECK(outstanding<=allocated)
);
CREATE INDEX storage_scan ON storage_material(status, expires_at);
ALTER TABLE runtime_evidence ADD COLUMN expired_at bigint;
ALTER TABLE runtime_evidence ADD COLUMN replacement text;
ALTER TABLE runtime_evidence ADD COLUMN retrospective text;
ALTER TABLE runtime_evidence ADD COLUMN cleanup_retry jsonb;

CREATE FUNCTION storage_phase_ended() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM pg_notify('storage_phase_ended','');
 RETURN NULL;
END $$;
CREATE TRIGGER storage_run_phase AFTER UPDATE OF state,phase,quiescent ON agent_run
 FOR EACH STATEMENT EXECUTE FUNCTION storage_phase_ended();
CREATE TRIGGER storage_preparation_phase AFTER INSERT ON preparation_history
 FOR EACH STATEMENT EXECUTE FUNCTION storage_phase_ended();
CREATE TRIGGER storage_validation_phase AFTER UPDATE OF stage,result ON candidate_validation
 FOR EACH STATEMENT EXECUTE FUNCTION storage_phase_ended();
CREATE TRIGGER storage_delivery_phase AFTER UPDATE OF state ON delivery_action
 FOR EACH STATEMENT EXECUTE FUNCTION storage_phase_ended();

-- Raw inputs are bounded before persistence, independently of model accounting.
CREATE FUNCTION storage_entry_limit() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE maximum bigint;
BEGIN
 SELECT (p.document->>'entry_bytes')::bigint INTO maximum
 FROM storage_guard g JOIN storage_policy p ON p.version=g.policy_version WHERE g.id=1;
 IF maximum IS NOT NULL AND pg_column_size(NEW)>maximum THEN
  RAISE EXCEPTION 'storage entry limit exceeded' USING ERRCODE='54000';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER storage_business_input BEFORE INSERT OR UPDATE ON business_request
 FOR EACH ROW EXECUTE FUNCTION storage_entry_limit();
CREATE TRIGGER storage_run_input BEFORE INSERT ON run_event
 FOR EACH ROW EXECUTE FUNCTION storage_entry_limit();
CREATE TRIGGER storage_preparation_input BEFORE INSERT ON preparation_history
 FOR EACH ROW EXECUTE FUNCTION storage_entry_limit();
CREATE TRIGGER storage_runtime_input BEFORE INSERT ON runtime_evidence_chunk
 FOR EACH ROW EXECUTE FUNCTION storage_entry_limit();
