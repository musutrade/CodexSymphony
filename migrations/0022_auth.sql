-- Platform identity is independent of Requirement/Run/GitHub business facts.
CREATE TABLE platform_account (
 username text PRIMARY KEY, password_hash text NOT NULL
);
CREATE TABLE platform_session (
 digest text PRIMARY KEY, username text REFERENCES platform_account(username),
 expires_at bigint NOT NULL, revoked boolean NOT NULL DEFAULT false
);
CREATE INDEX platform_session_account ON platform_session(username);
CREATE TABLE platform_login_limit (
 dimension text NOT NULL, digest text NOT NULL, window_start bigint NOT NULL,
 attempts integer NOT NULL, PRIMARY KEY(dimension,digest)
);
