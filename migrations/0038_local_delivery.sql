-- Keep the existing action/attempt/observation ledger. Local targets have no
-- GitHub identity; their internal repository and observed delivery are explicit.
ALTER TABLE delivery ADD COLUMN mode text NOT NULL DEFAULT 'github_pr'
 CHECK (mode IN ('github_pr','local_git'));
ALTER TABLE delivery ADD COLUMN internal_repository_id bigint REFERENCES repository(id);
ALTER TABLE delivery ADD COLUMN local_binding jsonb;
ALTER TABLE delivery ADD COLUMN local_storage_blocked boolean NOT NULL DEFAULT false;
ALTER TABLE delivery ADD COLUMN local_acceptance_started boolean NOT NULL DEFAULT false;
ALTER TABLE delivery ADD COLUMN local_acceptance jsonb;
ALTER TABLE delivery ADD COLUMN local_acceptance_job jsonb;
ALTER TABLE delivery ADD COLUMN local_acceptance_quiescent boolean NOT NULL DEFAULT false;
ALTER TABLE initial_run ADD COLUMN local_binding jsonb;
ALTER TABLE delivery ADD CONSTRAINT local_delivery_identity CHECK
 (mode <> 'local_git' OR (repository_id=0 AND pr_number IS NULL AND internal_repository_id IS NOT NULL AND local_binding IS NOT NULL));
-- Newly installed capabilities require explicit deployment authorization.
INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled)
 VALUES ('delivery:local_git','all','{}'::bigint[],false);

ALTER TABLE linked_failure ADD COLUMN local_delivery text REFERENCES delivery(action_key);
ALTER TABLE linked_failure ADD COLUMN local_binding jsonb;
ALTER TABLE linked_failure DROP CONSTRAINT linked_failure_check;
ALTER TABLE linked_failure ADD CONSTRAINT linked_failure_source CHECK
 (num_nonnulls(merge_key,integration_id,local_delivery)=1);
CREATE UNIQUE INDEX linked_local_failure ON linked_failure(local_delivery) WHERE local_delivery IS NOT NULL;
