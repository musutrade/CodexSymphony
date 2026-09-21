-- Historical observations remain GitHub facts, never business transitions.
CREATE TABLE github_evidence_history (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 repository_id bigint NOT NULL,
 pr_number bigint,
 observed_at bigint NOT NULL,
 policy jsonb NOT NULL,
 evidence jsonb NOT NULL
);
CREATE INDEX github_evidence_history_identity ON github_evidence_history(repository_id,pr_number,observed_at);

INSERT INTO github_evidence_history(repository_id,observed_at,policy,evidence)
 SELECT repository_id,checked_at,policy,capability FROM github_repository WHERE capability IS NOT NULL;
INSERT INTO github_evidence_history(repository_id,pr_number,observed_at,policy,evidence)
 SELECT repository_id,number,last_synced_at,observation->'policy',observation FROM github_pr WHERE observation IS NOT NULL;
-- Existing observations are retained, but must be revalidated by the new reader.
UPDATE github_repository SET stale=true,next_attempt_at=0;
UPDATE github_pr SET stale=true,next_attempt_at=0;
CREATE OR REPLACE VIEW github_pr_observation AS
 SELECT p.repository_id,p.number,p.requirement_id,p.observation,p.last_synced_at,
 (p.stale OR p.last_synced_at IS NULL OR p.last_synced_at <= extract(epoch FROM now())::bigint-60
  OR p.last_synced_at > extract(epoch FROM now())::bigint
  OR NOT EXISTS(SELECT 1 FROM github_repository g WHERE g.repository_id=p.repository_id AND g.policy=p.observation->'policy')) AS stale,
 p.error,p.failures,p.next_attempt_at FROM github_pr p;
