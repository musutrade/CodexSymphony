use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
pub async fn database() -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "delivery_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql(r#"
INSERT INTO repository VALUES(1,1,'{"github_repository_id":7,"remote":"owner/repo","base_branch":"main","revoked":false}',0);
INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1);
INSERT INTO requirement_revision VALUES(1,1,'{"repository_version":1,"repository":{"github_repository_id":7,"remote":"owner/repo","base_branch":"main"}}');
UPDATE execution_control SET requirement_id=1,recovery_complete=true,incarnation='boot';
INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('run',1,1,'boot','request','/tmp','owned','{}','Succeeded',true);
INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('run','{"head":"candidate","workspace":{"branch":"ai/req-1","baseline":"base"}}',true);
INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('validation',1,1,'run','candidate','tree','{}','[]','tree','tree','entry','entry','handoff','succeeded');
INSERT INTO validation_step(validation_id,step_id,command,status,consumer) VALUES('validation','test','[]','succeeded','handoff');
INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES(7,1,'{}',1,'{"blockers":[],"policy":{}}',extract(epoch FROM now())::bigint,false);
"#).execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    codexsymphony_server::delivery_store::enqueue(&mut tx, "validation")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    pool
}
