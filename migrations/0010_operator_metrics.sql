-- Measurement starts here; older rows cannot establish a zero-intervention rate.
CREATE TABLE operator_measurement_epoch (
 id integer PRIMARY KEY CHECK(id=1), started_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO operator_measurement_epoch(id) VALUES(1);
CREATE TABLE operator_intervention (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 reason text NOT NULL, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE operator_phase (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 run_id text REFERENCES agent_run(id),
 phase text NOT NULL, started_at timestamptz NOT NULL DEFAULT now(),
 finished_at timestamptz
);
CREATE FUNCTION record_operator_requirement() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.paused IS DISTINCT FROM OLD.paused THEN
  INSERT INTO operator_intervention(requirement_id,reason) VALUES(NEW.id,CASE WHEN NEW.paused THEN 'user_pause' ELSE 'user_resume' END);
 END IF;
 IF NEW.cancel_requested AND NOT OLD.cancel_requested THEN
  INSERT INTO operator_intervention(requirement_id,reason) VALUES(NEW.id,'user_cancel');
 END IF;
 IF NEW.state IS DISTINCT FROM OLD.state THEN
  IF OLD.state='Ready' AND NEW.state='Draft' THEN
   INSERT INTO operator_intervention(requirement_id,reason) VALUES(NEW.id,'review_withdrawal');
  END IF;
  UPDATE operator_phase SET finished_at=now() WHERE requirement_id=NEW.id AND run_id IS NULL AND finished_at IS NULL;
  IF NEW.state IN ('Ready','Running') THEN
   INSERT INTO operator_phase(requirement_id,phase) VALUES(NEW.id,NEW.state);
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER operator_requirement AFTER UPDATE ON requirement FOR EACH ROW EXECUTE FUNCTION record_operator_requirement();
CREATE FUNCTION record_operator_question() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 INSERT INTO operator_intervention(requirement_id,reason) VALUES(NEW.requirement_id,'human_question');
 RETURN NEW;
END $$;
CREATE TRIGGER operator_question AFTER INSERT ON runtime_question FOR EACH ROW EXECUTE FUNCTION record_operator_question();
CREATE FUNCTION record_operator_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NOT NEW.quiescent THEN
   INSERT INTO operator_phase(requirement_id,run_id,phase) VALUES(NEW.requirement_id,NEW.id,NEW.phase);
  END IF;
 ELSE
  IF NEW.phase IS DISTINCT FROM OLD.phase OR (NEW.quiescent AND NOT OLD.quiescent) THEN
   UPDATE operator_phase SET finished_at=now() WHERE run_id=NEW.id AND finished_at IS NULL;
   IF NOT NEW.quiescent THEN
    INSERT INTO operator_phase(requirement_id,run_id,phase) VALUES(NEW.requirement_id,NEW.id,NEW.phase);
   END IF;
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER operator_run AFTER INSERT OR UPDATE ON agent_run FOR EACH ROW EXECUTE FUNCTION record_operator_run();
