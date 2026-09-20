-- Empty/redundant controller updates must not continuously wake storage scans.
-- Actual phase transitions still notify after commit; periodic scans remain.
DROP TRIGGER storage_run_phase ON agent_run;
CREATE TRIGGER storage_run_phase AFTER UPDATE OF state,phase,quiescent ON agent_run
 FOR EACH ROW WHEN (OLD.state IS DISTINCT FROM NEW.state
  OR OLD.phase IS DISTINCT FROM NEW.phase OR OLD.quiescent IS DISTINCT FROM NEW.quiescent)
 EXECUTE FUNCTION storage_phase_ended();
DROP TRIGGER storage_preparation_phase ON preparation_history;
CREATE TRIGGER storage_preparation_phase AFTER INSERT ON preparation_history
 FOR EACH ROW EXECUTE FUNCTION storage_phase_ended();
DROP TRIGGER storage_validation_phase ON candidate_validation;
CREATE TRIGGER storage_validation_phase AFTER UPDATE OF stage,result ON candidate_validation
 FOR EACH ROW WHEN (OLD.stage IS DISTINCT FROM NEW.stage OR OLD.result IS DISTINCT FROM NEW.result)
 EXECUTE FUNCTION storage_phase_ended();
DROP TRIGGER storage_delivery_phase ON delivery_action;
CREATE TRIGGER storage_delivery_phase AFTER UPDATE OF state ON delivery_action
 FOR EACH ROW WHEN (OLD.state IS DISTINCT FROM NEW.state)
 EXECUTE FUNCTION storage_phase_ended();
