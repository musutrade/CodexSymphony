-- Append-only admission evidence; business/AgentRun/budget facts are unchanged.
CREATE TABLE environment_observation (
    id bigserial PRIMARY KEY,
    requirement_id bigint NOT NULL REFERENCES requirement(id),
    revision bigint NOT NULL,
    stage text NOT NULL,
    observed_at timestamptz NOT NULL DEFAULT now(),
    report jsonb NOT NULL
);
CREATE INDEX environment_observation_requirement ON environment_observation(requirement_id,revision,id);

-- Observe durable stage transitions without project-code instrumentation.
-- NULL on historical records means unknown; never invent a historical start.
ALTER TABLE candidate_validation ADD COLUMN started_at timestamptz;
ALTER TABLE candidate_validation ADD COLUMN finished_at timestamptz;
CREATE FUNCTION record_validation_timing() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.stage='validation' AND NEW.result='pending' AND NEW.started_at IS NULL AND
       (TG_OP='INSERT' OR OLD.stage IS DISTINCT FROM NEW.stage) THEN
        NEW.started_at=clock_timestamp();
    END IF;
    IF NEW.started_at IS NOT NULL AND NEW.finished_at IS NULL AND NEW.result<>'pending' THEN
        NEW.finished_at=clock_timestamp();
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER validation_timing BEFORE INSERT OR UPDATE OF stage,result ON candidate_validation
    FOR EACH ROW EXECUTE FUNCTION record_validation_timing();

ALTER TABLE delivery_attempt ADD COLUMN finished_at timestamptz;
CREATE FUNCTION record_delivery_timing() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.result IS NULL AND NEW.result IS NOT NULL AND NEW.finished_at IS NULL THEN
        NEW.finished_at=clock_timestamp();
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER delivery_timing BEFORE UPDATE OF result ON delivery_attempt
    FOR EACH ROW EXECUTE FUNCTION record_delivery_timing();
