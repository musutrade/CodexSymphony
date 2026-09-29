-- An operator correction starts a new acceptance invocation for the SAME
-- delivered version. Prior jobs/results and misclassified code failures remain
-- immutable history; no model reservation or delivery attempt is reset.
CREATE TABLE local_acceptance_recheck (
 request_id text PRIMARY KEY,
 delivery_key text NOT NULL REFERENCES delivery(action_key),
 ordinal integer NOT NULL CHECK(ordinal BETWEEN 1 AND 3),
 input jsonb NOT NULL CHECK(pg_column_size(input)<=65536),
 previous_job jsonb NOT NULL,
 previous_result jsonb NOT NULL,
 previous_failure jsonb NOT NULL,
 plan jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(delivery_key,ordinal)
);
REVOKE ALL ON local_acceptance_recheck FROM PUBLIC;
