-- Retain repository 1, the sole execution owner, all revisions and budgets.
ALTER TABLE repository DROP CONSTRAINT repository_id_check;
ALTER TABLE repository ADD CONSTRAINT repository_id_positive CHECK (id > 0);
CREATE UNIQUE INDEX repository_github_identity ON repository ((document->>'github_repository_id'));
ALTER TABLE requirement ADD COLUMN repository_id integer NOT NULL DEFAULT 1 CHECK (repository_id > 0);
