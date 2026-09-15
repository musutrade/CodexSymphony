CREATE TABLE repository (
 id integer PRIMARY KEY CHECK (id = 1), version bigint NOT NULL,
 document jsonb NOT NULL, revoked_through_version bigint NOT NULL DEFAULT 0
);
CREATE TABLE requirement (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 version bigint NOT NULL, state text NOT NULL CHECK (state IN ('Draft','Ready','Running','Submitted')),
 contract jsonb NOT NULL, revision bigint NOT NULL DEFAULT 0,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE requirement_revision (
 requirement_id bigint NOT NULL REFERENCES requirement(id), revision bigint NOT NULL,
 document jsonb NOT NULL, PRIMARY KEY (requirement_id, revision)
);
CREATE TABLE business_event (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 object_id text NOT NULL, kind text NOT NULL, version bigint NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE business_request (
 request_id text PRIMARY KEY, input jsonb NOT NULL, result jsonb NOT NULL
);
