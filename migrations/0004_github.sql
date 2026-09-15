-- External observations never rewrite Requirement, AgentRun, or validation.
CREATE TABLE github_repository (
 repository_id bigint PRIMARY KEY,
 repository_version bigint NOT NULL,
 policy jsonb NOT NULL,
 probe_pr bigint NOT NULL CHECK (probe_pr > 0),
 capability jsonb,
 checked_at bigint,
 stale boolean NOT NULL DEFAULT true,
 failures integer NOT NULL DEFAULT 0,
 next_attempt_at bigint NOT NULL DEFAULT 0,
 error jsonb
);
CREATE TABLE github_pr (
 repository_id bigint NOT NULL REFERENCES github_repository(repository_id),
 number bigint NOT NULL CHECK (number > 0),
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 observation jsonb,
 last_synced_at bigint,
 stale boolean NOT NULL DEFAULT true,
 failures integer NOT NULL DEFAULT 0,
 next_attempt_at bigint NOT NULL DEFAULT 0,
 error jsonb,
 PRIMARY KEY(repository_id,number)
);
-- Time-derived stale remains accurate after process downtime. The previous
-- successful snapshot is retained for display, never presented as a fresh fact.
CREATE VIEW github_pr_observation AS
 SELECT repository_id,number,requirement_id,observation,last_synced_at,
 (stale OR last_synced_at IS NULL OR last_synced_at <= extract(epoch FROM now())::bigint-60) AS stale,
 error,failures,next_attempt_at FROM github_pr;
CREATE VIEW github_repository_capability AS
 SELECT g.repository_id,g.repository_version,g.policy,g.capability,g.checked_at,
 (g.stale OR g.checked_at IS NULL OR g.checked_at <= extract(epoch FROM now())::bigint-60
  OR NOT EXISTS(SELECT 1 FROM repository r WHERE r.version=g.repository_version
    AND (r.document->>'github_repository_id')::bigint=g.repository_id
    AND NOT (r.document->>'revoked')::boolean)) AS stale,
 g.error,g.failures,g.next_attempt_at FROM github_repository g;
