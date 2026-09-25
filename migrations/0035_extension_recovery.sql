-- Additive resolution metadata; old failures, attempts and authorizations remain.
ALTER TABLE recovery_failure ADD COLUMN resolution jsonb;
ALTER TABLE recovery_failure ADD COLUMN resolution_state text;
ALTER TABLE recovery_failure ADD COLUMN successor_validation text REFERENCES candidate_validation(id);
ALTER TABLE candidate_validation ADD COLUMN superseded_by text REFERENCES candidate_validation(id);
ALTER TABLE candidate_validation ADD COLUMN extension_feedback jsonb;

-- One accepted explicit recovery per original failure; the immutable command is
-- also saved in business_request for replay. Attempts retain retry_of identities.
CREATE UNIQUE INDEX extension_recovery_successor ON recovery_failure(successor_validation)
 WHERE successor_validation IS NOT NULL;
