-- Retain undispatched restoration jobs across controller restarts without
-- changing their original stage or discarding the authoritative source snapshot.
ALTER TABLE runtime_resume_history
 DROP CONSTRAINT runtime_resume_history_status_check,
 ADD CONSTRAINT runtime_resume_history_status_check
 CHECK (status IN ('restoring', 'prepared'));
