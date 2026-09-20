-- Explicit operator recovery intent is independent of the user's pause intent.
ALTER TABLE agent_run ADD COLUMN storage_resume_requested boolean NOT NULL DEFAULT false;
