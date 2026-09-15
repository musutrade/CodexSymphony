-- Preparation is execution evidence, never a business completion or code repair.
CREATE TABLE preparation_record (
 run_id text PRIMARY KEY,
 requirement_id bigint NOT NULL,
 revision bigint NOT NULL,
 launch jsonb NOT NULL,
 retry jsonb NOT NULL,
 ready boolean NOT NULL DEFAULT false,
 evidence jsonb,
 checked_at bigint,
 FOREIGN KEY (requirement_id,revision) REFERENCES requirement_revision(requirement_id,revision)
);
CREATE TABLE preparation_history (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 run_id text NOT NULL REFERENCES preparation_record(run_id),
 recorded_at bigint NOT NULL,
 event jsonb NOT NULL
);
-- Latched independently of user pause. Only explicit recovery clears it.
CREATE TABLE storage_guard (
 id integer PRIMARY KEY CHECK(id=1),
 blocked boolean NOT NULL DEFAULT false,
 error text
);
INSERT INTO storage_guard(id) VALUES(1);
