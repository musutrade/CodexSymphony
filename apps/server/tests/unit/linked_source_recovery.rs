use super::*;

async fn fixture() -> (PgPool, std::path::PathBuf, Command) {
    let (pool, root, _, _, failure) = crate::linked_repair_worker::tests::setup().await;
    let evidence: crate::validation::ValidationEvidence =
        sqlx::query_scalar::<_, Value>("SELECT evidence FROM linked_failure WHERE id=$1")
            .bind(&failure)
            .fetch_one(&pool)
            .await
            .map(decode)
            .unwrap()
            .unwrap();
    let mut binding = crate::integration::Binding {
        requirement: 1,
        revision: 1,
        authorization: 1,
        input_sha256: "fixture".into(),
        versions: vec![crate::integration::Version {
            repository_id: 1,
            repository_version: 1,
            github_repository_id: 7,
            candidate: evidence.candidate.clone(),
            artifacts: vec![],
        }],
        trusted: evidence.trusted.clone(),
        required: vec!["test".into()],
    };
    let auth = crate::integration::Authorization {
        configuration_sha256: evidence.trusted.config_sha256.clone(),
        repositories: vec![crate::integration::Repository {
            repository_id: 1,
            repository_version: 1,
            selection: crate::integration::Selection::Fixed {
                sha: evidence.candidate.sha.clone(),
            },
            repair_scope: r#"{"schema":"linked-repair/v1","checks":{"test":["source"]}}"#.into(),
        }],
    };
    let input = json!({"child":{"kind":"validation_only"},"review":{"integration":auth}});
    binding.input_sha256 = crate::validation::sha256(serde_json::to_vec(&input).unwrap());
    crate::merge_test_support::reviewed_group(&pool, input).await;
    sqlx::query("INSERT INTO integration_validation(id,requirement_id,authorization_id,revision,binding,job,launch,state,quiescent,result) VALUES('source-recheck-original',1,1,1,$1,'{}','{}','failed',true,$2)")
        .bind(json!(binding)).bind(json!({"evidence":evidence})).execute(&pool).await.unwrap();
    sqlx::query("UPDATE linked_failure SET merge_key=NULL,integration_id='source-recheck-original',state='blocked',blocker='failure outside authorized repair checks',baseline=NULL,repository_id=NULL,source_run=NULL WHERE id=$1")
        .bind(&failure).execute(&pool).await.unwrap();
    sqlx::raw_sql("UPDATE requirement SET state='Running'; UPDATE execution_control SET paused=true,recovery_complete=true; UPDATE agent_run SET quiescent=true; UPDATE workspace_operation SET status='complete'; UPDATE storage_guard SET blocked=false;")
        .execute(&pool).await.unwrap();
    let (version,budget_version): (i64,i64) = sqlx::query_as("SELECT r.version,b.version FROM requirement r JOIN requirement_budget b ON b.requirement_id=r.id WHERE r.id=1").fetch_one(&pool).await.unwrap();
    let command = Command {
        request_id: "source-recheck-fixture".into(),
        requirement_id: 1,
        version,
        revision: 1,
        failure_id: failure,
        budget_version,
        configuration_sha256: evidence.trusted.config_sha256,
        reason: "reviewed collector-compatible source repair; same original account".into(),
    };
    (pool, root, command)
}
async fn preserved(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('budget',(SELECT to_jsonb(t) FROM requirement_budget t WHERE requirement_id=1),'grants',(SELECT jsonb_agg(to_jsonb(t)) FROM budget_authorization t),'calls',(SELECT jsonb_agg(to_jsonb(t)) FROM model_call t),'reservations',(SELECT jsonb_agg(to_jsonb(t)) FROM repair_reservation t),'failure',(SELECT jsonb_build_object('evidence',evidence,'required',required_steps,'attempts',source_attempts,'integration',integration_id,'created_at',created_at) FROM linked_failure LIMIT 1),'artifacts',(SELECT jsonb_agg(to_jsonb(t)) FROM diagnostic_artifact t))")
        .fetch_one(pool).await.unwrap()
}
#[tokio::test]
async fn paused_original_source_recheck_is_idempotent_and_preserves_paid_history() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, command) = fixture().await;
    let before = preserved(&pool).await;
    let result = recheck(&pool, &command).await.unwrap();
    assert_eq!(result["started"], false);
    assert_eq!(result["version"], command.version + 1);
    assert_eq!(recheck(&pool, &command).await.unwrap(), result);
    crate::budget_admin::dispatch(
        &["repair-source-recheck".into()],
        &serde_json::to_vec(&command).unwrap(),
        &pool,
    )
    .await
    .unwrap();
    assert!(
        crate::budget_admin::dispatch(&["repair-source-recheck".into()], b"{}", &pool)
            .await
            .is_err()
    );
    assert_eq!(preserved(&pool).await, before);
    let (state, receipts): (String, Value) =
        sqlx::query_as("SELECT state,source_receipts FROM linked_failure WHERE id=$1")
            .bind(&command.failure_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "observed");
    assert_eq!(
        receipts[0]["previous_blocker"],
        "failure outside authorized repair checks"
    );
    let mut changed = command.clone();
    changed.reason = "different bytes".into();
    assert!(recheck(&pool, &changed).await.is_err());
    changed = command.clone();
    changed.request_id = "another-recheck".into();
    assert!(recheck(&pool, &changed).await.is_err());
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn invalid_or_unbound_source_rechecks_leave_original_failure_closed() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, command) = fixture().await;
    let before = preserved(&pool).await;
    for invalid in [
        Command {
            request_id: String::new(),
            ..command.clone()
        },
        Command {
            reason: " ".into(),
            ..command.clone()
        },
        Command {
            configuration_sha256: "short".into(),
            ..command.clone()
        },
        Command {
            configuration_sha256: "z".repeat(64),
            ..command.clone()
        },
        Command {
            version: command.version + 1,
            ..command.clone()
        },
        Command {
            revision: 2,
            ..command.clone()
        },
        Command {
            budget_version: command.budget_version + 1,
            ..command.clone()
        },
        Command {
            configuration_sha256: "0".repeat(64),
            ..command.clone()
        },
    ] {
        assert!(recheck(&pool, &invalid).await.is_err());
    }
    for (change, restore) in [
        (
            "UPDATE execution_control SET paused=false",
            "UPDATE execution_control SET paused=true",
        ),
        (
            "UPDATE execution_control SET recovery_complete=false",
            "UPDATE execution_control SET recovery_complete=true",
        ),
        (
            "UPDATE storage_guard SET blocked=true",
            "UPDATE storage_guard SET blocked=false",
        ),
        (
            "UPDATE group_execution_item SET frozen=true",
            "UPDATE group_execution_item SET frozen=false",
        ),
        (
            "UPDATE group_queue SET state='needs_review'",
            "UPDATE group_queue SET state='waiting_scheduler'",
        ),
        (
            "UPDATE agent_run SET quiescent=false",
            "UPDATE agent_run SET quiescent=true",
        ),
        (
            "UPDATE integration_validation SET quiescent=false",
            "UPDATE integration_validation SET quiescent=true",
        ),
    ] {
        sqlx::query(change).execute(&pool).await.unwrap();
        assert!(recheck(&pool, &command).await.is_err());
        sqlx::query(restore).execute(&pool).await.unwrap();
    }
    let input: Value = sqlx::query_scalar("SELECT input FROM group_execution_item LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut bad = input.clone();
    bad["review"]["integration"]["repositories"][0]["repair_scope"] = json!("unreviewed prose");
    sqlx::query("UPDATE group_execution_item SET input=$1")
        .bind(bad)
        .execute(&pool)
        .await
        .unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    sqlx::query("UPDATE group_execution_item SET input=$1")
        .bind(input)
        .execute(&pool)
        .await
        .unwrap();
    let binding: Value = sqlx::query_scalar("SELECT binding FROM integration_validation LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    for field in ["required", "versions", "input_sha256", "candidate"] {
        let mut bad = binding.clone();
        match field {
            "required" => bad["required"] = json!(["other"]),
            "versions" => bad["versions"] = json!([]),
            "candidate" => bad["versions"][0]["candidate"]["sha"] = json!("different-source"),
            _ => bad["input_sha256"] = json!("different-input"),
        }
        sqlx::query("UPDATE integration_validation SET binding=$1")
            .bind(bad)
            .execute(&pool)
            .await
            .unwrap();
        assert!(recheck(&pool, &command).await.is_err());
    }
    sqlx::query("UPDATE integration_validation SET binding=$1")
        .bind(binding)
        .execute(&pool)
        .await
        .unwrap();
    let original: Value = sqlx::query_scalar("SELECT result FROM integration_validation LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE integration_validation SET result='{}'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    sqlx::query("UPDATE integration_validation SET result=$1")
        .bind(original)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(preserved(&pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM linked_failure LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "blocked"
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
