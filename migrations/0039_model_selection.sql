-- Additive evidence only. Frozen choices live in immutable review snapshots.
CREATE TABLE runtime_model_identity (
    run_id text PRIMARY KEY REFERENCES agent_run(id),
    frozen jsonb NOT NULL,
    actual jsonb NOT NULL,
    matched boolean NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
