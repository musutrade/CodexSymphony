-- Ordering and pending review are mutable control facts, never Run inputs.
ALTER TABLE group_execution_item ADD COLUMN queue_order bigint;
UPDATE group_execution_item SET queue_order=(input#>>'{child,order}')::bigint;
ALTER TABLE group_execution_item ADD COLUMN frozen boolean NOT NULL DEFAULT false;
ALTER TABLE group_execution_item ADD COLUMN removed boolean NOT NULL DEFAULT false;
ALTER TABLE group_execution_item ADD COLUMN authorized_draft_revision bigint;
ALTER TABLE group_execution_item ADD COLUMN authorized_review_version bigint;
CREATE TABLE group_edit (
 draft_id text PRIMARY KEY REFERENCES imported_draft(id),
 version bigint NOT NULL,
 document jsonb NOT NULL,
 review jsonb NOT NULL,
 affected jsonb NOT NULL,
 repositories jsonb NOT NULL
);
CREATE TABLE group_queue_event (
 request_id text PRIMARY KEY,
 draft_id text NOT NULL REFERENCES imported_draft(id),
 input jsonb NOT NULL,
 result jsonb NOT NULL,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE OR REPLACE VIEW execution_queue AS
 SELECT r.id AS requirement_id,r.revision,r.paused,r.created_at AS queued_at,
 'requirement'::text AS queue_key,r.id AS item_order
 FROM requirement r WHERE r.state='Ready' AND NOT EXISTS(SELECT 1 FROM group_execution_item i WHERE i.requirement_id=r.id)
 UNION ALL
 SELECT i.requirement_id,r.revision,COALESCE(r.paused OR r.state<>'Ready',false) OR i.frozen,q.created_at,
 'group:'||q.draft_id,COALESCE(i.queue_order,(i.input#>>'{child,order}')::bigint)
 FROM group_execution_item i JOIN group_queue q USING(draft_id)
 LEFT JOIN requirement r ON r.id=i.requirement_id
 WHERE NOT i.removed AND NOT EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id);
