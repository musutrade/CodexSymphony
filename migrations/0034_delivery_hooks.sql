-- Delivery hooks share the existing supervised invocation ledger. Retain the
-- exact input across restart; never reconstruct an unknown call from defaults.
ALTER TABLE project_hook_invocation ADD COLUMN input jsonb;
