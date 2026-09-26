-- Scope is deployment-owned admission, independent of plugin selection.
CREATE TABLE plugin_scope (
 plugin_id text PRIMARY KEY CHECK (length(plugin_id)>0),
 kind text NOT NULL CHECK (kind IN ('all','repositories')),
 repository_ids bigint[] NOT NULL,
 enabled boolean NOT NULL,
 version bigint NOT NULL DEFAULT 1 CHECK (version>0),
 CHECK ((kind='all' AND cardinality(repository_ids)=0) OR
        (kind='repositories' AND cardinality(repository_ids)>0))
);
CREATE TABLE plugin_scope_history (
 plugin_id text NOT NULL,
 version bigint NOT NULL,
 kind text NOT NULL,
 repository_ids bigint[] NOT NULL,
 enabled boolean NOT NULL,
 PRIMARY KEY(plugin_id,version)
);
CREATE FUNCTION plugin_scope_validate() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target bigint;
BEGIN
 IF TG_OP='UPDATE' AND NEW.plugin_id<>OLD.plugin_id THEN
  RAISE EXCEPTION 'plugin identity is immutable';
 END IF;
 IF array_ndims(NEW.repository_ids)>1 OR array_position(NEW.repository_ids,NULL) IS NOT NULL
 OR cardinality(NEW.repository_ids)<>(SELECT count(DISTINCT id) FROM unnest(NEW.repository_ids) id) THEN
  RAISE EXCEPTION 'invalid repository scope';
 END IF;
 FOREACH target IN ARRAY NEW.repository_ids LOOP
  PERFORM 1 FROM repository WHERE id=target FOR KEY SHARE;
  IF NOT FOUND THEN RAISE EXCEPTION 'unknown repository in plugin scope'; END IF;
 END LOOP;
 IF TG_OP='INSERT' THEN NEW.version:=1; ELSE NEW.version:=OLD.version+1; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER plugin_scope_validate BEFORE INSERT OR UPDATE ON plugin_scope
 FOR EACH ROW EXECUTE FUNCTION plugin_scope_validate();
CREATE FUNCTION plugin_scope_retain() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 INSERT INTO plugin_scope_history VALUES(NEW.plugin_id,NEW.version,NEW.kind,NEW.repository_ids,NEW.enabled);
 RETURN NEW;
END $$;
CREATE TRIGGER plugin_scope_retain AFTER INSERT OR UPDATE ON plugin_scope
 FOR EACH ROW EXECUTE FUNCTION plugin_scope_retain();
CREATE FUNCTION plugin_scope_allows(plugin text, repo bigint) RETURNS boolean
LANGUAGE sql STABLE AS $$
 SELECT EXISTS(SELECT 1 FROM plugin_scope WHERE plugin_id=plugin AND enabled
 AND repo IS NOT NULL AND repo>0 AND (kind='all' OR repo=ANY(repository_ids)))
$$;
-- Explicit compatibility registrations. New plugins receive no default scope.
INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES
 ('agent:codex','all','{}'::bigint[],true),('validation:native','all','{}'::bigint[],true),('delivery:github','all','{}'::bigint[],true);
INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled)
 SELECT 'notification:'||id,'all','{}'::bigint[],enabled FROM notification_plugin;
-- Preserve previously reviewed hooks, grouped by their existing stable name.
INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled)
 SELECT DISTINCT 'hook:'||(hook->>'name'),'all','{}'::bigint[],true
 FROM (SELECT document->'hooks' AS hooks FROM repository UNION ALL SELECT document#>'{repository,hooks}' FROM execution_revision) reviewed, jsonb_array_elements(COALESCE(NULLIF(hooks,'null'::jsonb),'[]')) hook
 ON CONFLICT DO NOTHING;

CREATE TABLE plugin_scope_invocation (
 plugin_id text NOT NULL,
 invocation_id text NOT NULL,
 repository_id bigint NOT NULL,
 requirement_id bigint NOT NULL REFERENCES requirement(id),
 revision bigint NOT NULL,
 scope_version bigint NOT NULL,
 PRIMARY KEY(plugin_id,invocation_id),
 FOREIGN KEY(plugin_id,scope_version) REFERENCES plugin_scope_history(plugin_id,version)
);
CREATE FUNCTION plugin_scope_admit(plugin text, invocation text, task bigint, rev bigint, repo bigint)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE policy plugin_scope%ROWTYPE; original plugin_scope_invocation%ROWTYPE;
BEGIN
 IF task IS NULL OR rev IS NULL OR repo IS NULL OR invocation IS NULL OR invocation='' THEN
  RAISE EXCEPTION 'plugin invocation identity missing';
 END IF;
 SELECT * INTO policy FROM plugin_scope WHERE plugin_id=plugin FOR SHARE;
 IF NOT FOUND OR NOT plugin_scope_allows(plugin,repo) THEN
  RAISE EXCEPTION 'plugin repository scope unavailable: %',plugin;
 END IF;
 IF EXISTS(SELECT 1 FROM plugin_scope_invocation WHERE plugin_id=plugin AND invocation_id=invocation
 AND (requirement_id<>task OR revision<>rev OR repository_id<>repo)) THEN RAISE EXCEPTION 'plugin invocation identity changed'; END IF;
 INSERT INTO plugin_scope_invocation VALUES(plugin,invocation,repo,task,rev,policy.version) ON CONFLICT DO NOTHING;
 SELECT * INTO original FROM plugin_scope_invocation WHERE plugin_id=plugin AND invocation_id=invocation;
 IF original.requirement_id<>task OR original.revision<>rev OR original.repository_id<>repo THEN
  RAISE EXCEPTION 'plugin invocation identity changed';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM plugin_scope_history WHERE plugin_id=plugin AND version=original.scope_version
 AND enabled AND (kind='all' OR repo=ANY(repository_ids))) THEN RAISE EXCEPTION 'frozen plugin scope denied'; END IF;
END $$;
CREATE FUNCTION plugin_scope_repository(task bigint, rev bigint) RETURNS bigint LANGUAGE sql STABLE AS $$
 SELECT COALESCE((v.document->>'repository_id')::bigint,r.repository_id::bigint)
 FROM requirement r LEFT JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=rev WHERE r.id=task
$$;
ALTER TABLE lifecycle_event ADD COLUMN repository_id bigint;
UPDATE lifecycle_event SET repository_id=plugin_scope_repository(requirement_id,revision);
ALTER TABLE notification_plugin ADD CONSTRAINT notification_scope_registration
 CHECK (length(id)>0);
CREATE FUNCTION notification_scope_registration() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM 1 FROM plugin_scope WHERE plugin_id='notification:'||NEW.id;
 IF NOT FOUND THEN RAISE EXCEPTION 'explicit notification scope registration required'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER notification_scope_registration BEFORE INSERT OR UPDATE ON notification_plugin
 FOR EACH ROW EXECUTE FUNCTION notification_scope_registration();
ALTER TABLE notification_delivery ADD COLUMN scope_version bigint;
UPDATE notification_delivery d SET scope_version=s.version FROM plugin_scope s WHERE s.plugin_id='notification:'||d.plugin_id;
ALTER TABLE notification_delivery ALTER COLUMN scope_version SET NOT NULL;
CREATE FUNCTION notification_scope_allowed(plugin text, event bigint) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT EXISTS(SELECT 1 FROM lifecycle_event e JOIN notification_delivery d ON d.event_id=e.id
 JOIN plugin_scope_history s ON s.plugin_id='notification:'||d.plugin_id AND s.version=d.scope_version
 WHERE e.id=event AND d.plugin_id=plugin AND plugin_scope_allows(s.plugin_id,e.repository_id)
 AND s.enabled AND (s.kind='all' OR e.repository_id=ANY(s.repository_ids)))
$$;
CREATE OR REPLACE FUNCTION lifecycle_append(task bigint, rev bigint, phase_name text, source text, data jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE event_id bigint; ordinal bigint; repo bigint;
BEGIN
 PERFORM 1 FROM requirement WHERE id=task FOR UPDATE;
 IF NOT FOUND THEN RETURN; END IF;
 repo:=plugin_scope_repository(task,rev);
 SELECT COALESCE(max(sequence),0)+1 INTO ordinal FROM lifecycle_event WHERE requirement_id=task;
 INSERT INTO lifecycle_event(requirement_id,revision,sequence,phase,source_id,facts,repository_id)
 VALUES(task,rev,ordinal,phase_name,source,data,repo) RETURNING id INTO event_id;
 PERFORM 1 FROM plugin_scope WHERE plugin_id LIKE 'notification:%' ORDER BY plugin_id FOR SHARE;
 INSERT INTO notification_delivery(plugin_id,event_id,scope_version)
 SELECT p.id,event_id,s.version FROM notification_plugin p JOIN plugin_scope s ON s.plugin_id='notification:'||p.id
 WHERE p.enabled AND plugin_scope_allows(s.plugin_id,repo);
END $$;
CREATE OR REPLACE FUNCTION notification_claim(plugin text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path FROM CURRENT AS $$
DECLARE delivery notification_delivery%ROWTYPE; event lifecycle_event%ROWTYPE;
BEGIN
 PERFORM 1 FROM plugin_scope WHERE plugin_id='notification:'||plugin FOR SHARE;
 IF NOT EXISTS(SELECT 1 FROM notification_plugin WHERE id=plugin AND database_role=session_user AND enabled) THEN
  RAISE EXCEPTION 'notification plugin not authorized';
 END IF;
 UPDATE notification_delivery SET state='failed',last_result='retry_exhausted'
 WHERE plugin_id=plugin AND state IN ('pending','unknown') AND
 (deadline<=clock_timestamp() OR (attempts>=attempt_limit AND next_attempt_at<=clock_timestamp()));
 SELECT * INTO delivery FROM notification_delivery d WHERE plugin_id=plugin
 AND notification_scope_allowed(plugin,d.event_id)
 AND state IN ('pending','unknown') AND attempts<attempt_limit AND next_attempt_at<=clock_timestamp()
 AND deadline>clock_timestamp()+interval '15 seconds'
 AND NOT EXISTS(SELECT 1 FROM notification_delivery earlier JOIN lifecycle_event a ON a.id=earlier.event_id
 JOIN lifecycle_event b ON b.id=d.event_id WHERE earlier.plugin_id=plugin AND earlier.state IN ('pending','unknown')
 AND notification_scope_allowed(plugin,earlier.event_id)
 AND a.requirement_id=b.requirement_id AND a.sequence<b.sequence)
 ORDER BY event_id FOR UPDATE SKIP LOCKED LIMIT 1;
 IF NOT FOUND THEN RETURN NULL; END IF;
 UPDATE notification_delivery SET state='unknown',attempts=attempts+1,
 next_attempt_at=clock_timestamp()+CASE WHEN attempts=0 THEN interval '30 seconds' ELSE interval '120 seconds' END,
 last_result='unconfirmed' WHERE plugin_id=plugin AND event_id=delivery.event_id;
 INSERT INTO notification_attempt(plugin_id,event_id,ordinal) VALUES(plugin,delivery.event_id,delivery.attempts+1);
 SELECT * INTO event FROM lifecycle_event WHERE id=delivery.event_id;
 RETURN jsonb_build_object('protocol_version',1,'plugin_id',plugin,'event_id',event.id,
 'repository_id',event.repository_id,'scope_version',delivery.scope_version,'attempt',delivery.attempts+1,'requirement_id',event.requirement_id,'revision',event.revision,
 'sequence',event.sequence,'phase',event.phase,'source_id',event.source_id,'facts',event.facts,
 'occurred_at',event.occurred_at,'detail_path','/requirements/'||event.requirement_id);
END $$;

REVOKE ALL ON plugin_scope,plugin_scope_history,plugin_scope_invocation FROM PUBLIC;
REVOKE ALL ON FUNCTION plugin_scope_allows(text,bigint),plugin_scope_admit(text,text,bigint,bigint,bigint),plugin_scope_repository(bigint,bigint),notification_scope_allowed(text,bigint) FROM PUBLIC;

CREATE FUNCTION notification_replay(task bigint, event bigint, plugin text) RETURNS boolean LANGUAGE plpgsql AS $$
DECLARE changed integer;
BEGIN
 PERFORM 1 FROM plugin_scope WHERE plugin_id='notification:'||plugin FOR SHARE;
 PERFORM 1 FROM notification_plugin WHERE id=plugin AND enabled FOR SHARE;
 IF NOT FOUND THEN RETURN false; END IF;
 UPDATE notification_delivery d SET state='pending',attempt_limit=6,next_attempt_at=clock_timestamp(),
 deadline=clock_timestamp()+interval '10 minutes',last_result='operator_replay'
 FROM lifecycle_event e WHERE e.id=d.event_id AND e.requirement_id=task AND d.event_id=event
 AND d.plugin_id=plugin AND notification_scope_allowed(plugin,event) AND d.state='failed' AND d.attempt_limit=3;
 GET DIAGNOSTICS changed=ROW_COUNT;
 RETURN changed=1;
END $$;
REVOKE ALL ON FUNCTION notification_replay(bigint,bigint,text) FROM PUBLIC;

-- Reviewed host scope references must name existing repository identities.
-- The repository being configured may be created in this same statement.
CREATE FUNCTION repository_extension_scope_validate() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE registration jsonb; scope text; targets bigint[]; target bigint;
BEGIN
 IF NEW.document->>'environment' IS NULL THEN RETURN NEW; END IF;
 FOR registration IN SELECT * FROM jsonb_array_elements((NEW.document->>'environment')::jsonb#>'{controlled,extensions}') LOOP
  scope:=registration->>'scope_ref';
  IF scope='all' THEN CONTINUE; END IF;
  IF scope IS NULL OR scope !~ '^repository:[1-9][0-9]*$|^repositories:[1-9][0-9]*(,[1-9][0-9]*)*$' THEN
   RAISE EXCEPTION 'invalid controlled repository scope';
  END IF;
  targets:=string_to_array(split_part(scope,':',2),',')::bigint[];
  IF cardinality(targets)<>(SELECT count(DISTINCT id) FROM unnest(targets) id) THEN
   RAISE EXCEPTION 'duplicate controlled repository scope';
  END IF;
  FOREACH target IN ARRAY targets LOOP
   IF target<>NEW.id THEN
    PERFORM 1 FROM repository WHERE id=target FOR KEY SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'unknown controlled repository scope'; END IF;
   END IF;
  END LOOP;
 END LOOP;
 RETURN NEW;
END $$;
CREATE TRIGGER repository_extension_scope_validate BEFORE INSERT OR UPDATE OF document ON repository
 FOR EACH ROW EXECUTE FUNCTION repository_extension_scope_validate();

-- Explicitly place pg_temp last: a notifier may have TEMP privilege on the
-- database, but its same-named tables must never shadow authorization tables.
DO $$
BEGIN
 EXECUTE format('ALTER FUNCTION notification_claim(text) SET search_path TO %I, pg_temp',current_schema());
 EXECUTE format('ALTER FUNCTION notification_ack(text,bigint,integer,text) SET search_path TO %I, pg_temp',current_schema());
END $$;
