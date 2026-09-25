-- Existing frozen validations keep their original protocol and evidence.
ALTER TABLE candidate_validation ADD COLUMN hook_context jsonb;
ALTER TABLE candidate_validation ADD COLUMN hook_evaluation jsonb;
ALTER TABLE candidate_validation ADD COLUMN hook_required boolean NOT NULL DEFAULT false;
ALTER TABLE candidate_validation ADD COLUMN hook_invalidated boolean NOT NULL DEFAULT false;

-- A later resume cannot revive proof invalidated by a control interruption.
CREATE FUNCTION invalidate_validation_hook() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME='requirement' THEN
        IF (NEW.paused AND NOT OLD.paused) OR (NEW.cancel_requested AND NOT OLD.cancel_requested) THEN
            UPDATE candidate_validation SET hook_invalidated=true
            WHERE requirement_id=NEW.id AND hook_required;
        END IF;
    ELSIF NEW.paused AND NOT OLD.paused THEN
        UPDATE candidate_validation SET hook_invalidated=true WHERE hook_required;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER invalidate_requirement_validation_hook AFTER UPDATE OF paused,cancel_requested ON requirement
FOR EACH ROW EXECUTE FUNCTION invalidate_validation_hook();
CREATE TRIGGER invalidate_global_validation_hook AFTER UPDATE OF paused ON execution_control
FOR EACH ROW EXECUTE FUNCTION invalidate_validation_hook();
