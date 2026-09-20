-- Child identity bridges group authorization to the existing single owner.
CREATE TABLE group_execution_item (
 draft_id text NOT NULL REFERENCES imported_draft(id),
 child_id text NOT NULL,
 authorization_id bigint NOT NULL REFERENCES group_authorization(id),
 requirement_id bigint UNIQUE REFERENCES requirement(id),
 input jsonb NOT NULL,
 PRIMARY KEY(draft_id,child_id)
);
CREATE TABLE group_completion (
 requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
 authorization_id bigint NOT NULL REFERENCES group_authorization(id),
 fact jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE group_claim_input (
 requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
 authorization_id bigint NOT NULL REFERENCES group_authorization(id),
 baseline text NOT NULL,
 dependencies jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE VIEW execution_queue AS
 SELECT r.id AS requirement_id,r.revision,r.paused,r.created_at AS queued_at,
 'requirement'::text AS queue_key,r.id AS item_order
 FROM requirement r WHERE r.state='Ready' AND NOT EXISTS(SELECT 1 FROM group_execution_item i WHERE i.requirement_id=r.id)
 UNION ALL
 SELECT i.requirement_id,r.revision,COALESCE(r.paused OR r.state<>'Ready',false),q.created_at,
 'group:'||q.draft_id,(i.input#>>'{child,order}')::bigint
 FROM group_execution_item i JOIN group_queue q USING(draft_id)
 LEFT JOIN requirement r ON r.id=i.requirement_id
 WHERE NOT EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id);
-- Cumulative attribution preserves pre-existing group usage/reservations.
CREATE TABLE group_accounted (
 requirement_id bigint PRIMARY KEY REFERENCES requirement(id),
 used jsonb NOT NULL DEFAULT '{"tokens":0,"turns":0,"model_seconds":0}',
 reserved jsonb NOT NULL DEFAULT '{"tokens":0,"turns":0,"model_seconds":0}'
);
