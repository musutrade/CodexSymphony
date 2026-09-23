pub(crate) mod client;
#[allow(dead_code)]
#[path = "../support/automatic_merge.rs"]
pub(crate) mod fixture;

/// Synthetic metadata for internal transaction tests, not a real authorization.
pub(crate) async fn reviewed_group(pool: &sqlx::PgPool, input: serde_json::Value) {
    sqlx::raw_sql("INSERT INTO imported_draft(id,version,document,source,source_sha256) VALUES('draft-boundary',1,'{}','{}','fixture');
        INSERT INTO imported_draft_revision(draft_id,version,document,source,source_sha256) VALUES('draft-boundary',1,'{}','{}','fixture');
        INSERT INTO group_review VALUES('draft-boundary',1,1,'{}');
        INSERT INTO group_review_revision(draft_id,version,draft_revision,document) VALUES('draft-boundary',1,1,'{}');
        INSERT INTO group_authorization(draft_id,review_version,request_id,input,snapshot) VALUES('draft-boundary',1,'boundary-fixture','{}','{}');
        INSERT INTO group_queue(draft_id,authorization_id,state) VALUES('draft-boundary',1,'waiting_scheduler');")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO group_execution_item(draft_id,child_id,authorization_id,requirement_id,input,authorized_draft_revision,authorized_review_version) VALUES('draft-boundary','C1',1,1,$1,1,1)")
        .bind(input).execute(pool).await.unwrap();
}
