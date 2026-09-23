-- A cancelled child is not completion evidence. Once its cleanup is proven,
-- remove only its execution slot; dependent children still require completion.
CREATE OR REPLACE VIEW execution_queue AS
 SELECT r.id AS requirement_id,r.revision,r.paused,r.created_at AS queued_at,
 'requirement'::text AS queue_key,r.id AS item_order
 FROM requirement r WHERE r.state='Ready' AND NOT EXISTS(SELECT 1 FROM group_execution_item i WHERE i.requirement_id=r.id)
 UNION ALL
 SELECT i.requirement_id,r.revision,COALESCE(r.paused OR r.state<>'Ready',false) OR i.frozen,q.created_at,
 'group:'||q.draft_id,COALESCE(i.queue_order,(i.input#>>'{child,order}')::bigint)
 FROM group_execution_item i JOIN group_queue q USING(draft_id)
 LEFT JOIN requirement r ON r.id=i.requirement_id
 WHERE NOT i.removed
 AND NOT COALESCE(r.cancel_requested AND r.cleanup_complete,false)
 AND NOT EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id);
