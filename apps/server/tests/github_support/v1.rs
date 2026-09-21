use super::*;
use codexsymphony_server::github_contract::*;

fn contract() -> DeliveryContract {
    let pre = RequiredCheck {
        selector: action_policy().required.remove(0),
        applicability: "always".into(),
        trigger: Trigger::PullRequest,
        job: Some("build".into()),
    };
    let mut post = pre.clone();
    post.trigger = Trigger::Push;
    if let Source::Actions { event, branch, .. } = &mut post.selector.source {
        *event = "push".into();
        *branch = "main".into();
    }
    DeliveryContract {
        schema_version: 1,
        pre_merge: PreMerge {
            source: PreMergeSource::Head,
            checkout: PreMergeSource::Head,
            checks: vec![pre],
            wait_seconds: 30,
        },
        post_merge: PostMerge::Checks {
            checks: vec![post],
            wait_seconds: 40,
            probe_pr: 2,
        },
        actions: Actions {
            rerun_actions: false,
            rerequest_checks: false,
            merge: false,
            merge_method: None,
            read_logs: false,
        },
        protection: json!({"required_status_checks":{"strict":true,"checks":[{"context":"ci","app_id":42}]},"enforce_admins":{"enabled":true}}),
        rules: Vec::new(),
    }
}
fn policy_v1() -> Policy {
    let mut p = action_policy();
    p.delivery = Some(contract());
    p
}
pub(super) fn grants(f: &Fixture, p: &Policy) {
    let expected = permissions(p);
    f.put("expected-permissions", expected.clone());
    f.grant(expected);
}
pub(super) async fn fixture() -> (Fixture, Policy) {
    let f = Fixture::new().await;
    let p = policy_v1();
    grants(&f, &p);
    f.actions();
    let mut j = job();
    j["name"] = json!("build");
    f.put(
        "/repos/owner/repo/actions/runs/10/attempts/2/jobs",
        json!({"jobs":[j.clone()]}),
    );
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E/protection",
        p.delivery.as_ref().unwrap().protection.clone(),
    );
    let mut merged = pr();
    merged["merged"] = json!(true);
    merged["number"] = json!(2);
    merged["merge_commit_sha"] = json!("merged");
    f.put("/repos/owner/repo/pulls/2", merged);
    f.put(
        "/repos/owner/repo/commits/merged/check-suites",
        json!({"check_suites":[{"id":19}]}),
    );
    let mut check = check(22, "success");
    check["head_sha"] = json!("merged");
    check["check_suite"]["id"] = json!(19);
    f.put(
        "/repos/owner/repo/check-suites/19/check-runs",
        json!({"check_runs":[check]}),
    );
    let mut r = run();
    r["id"] = json!(11);
    r["head_sha"] = json!("merged");
    r["event"] = json!("push");
    r["head_branch"] = json!("main");
    r["check_suite_id"] = json!(19);
    f.put(
        "/repos/owner/repo/actions/runs?head_sha=merged&per_page=100&page=1",
        json!({"workflow_runs":[r]}),
    );
    j["id"] = json!(21);
    j["run_id"] = json!(11);
    j["check_run_url"] = json!("https://api.github.com/repos/owner/repo/check-runs/22");
    f.put(
        "/repos/owner/repo/actions/runs/11/attempts/2/jobs",
        json!({"jobs":[j]}),
    );
    (f, p)
}
#[tokio::test]
async fn phases_private_public_and_no_combined_status() {
    let (f, p) = fixture().await;
    let now = github_service::now();
    for private in [true, false] {
        f.put("/repos/owner/repo", json!({"id":99,"full_name":"owner/repo","default_branch":"main","archived":false,"private":private}));
        let capability = github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .unwrap();
        assert!(capability.blockers.is_empty(), "{:?}", capability.blockers);
        assert_eq!(
            capability.configuration["post_merge_probe"]["merged_sha"],
            "merged"
        );
    }
    let observed = github_observe::observe(&mut f.client(), &p, 2, now)
        .await
        .unwrap();
    assert_eq!(
        observed.phases.as_ref().unwrap()[0].check_sha.as_deref(),
        Some("abc")
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].check_sha.as_deref(),
        Some("merged")
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].checks[0].state,
        CheckState::Success
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].actual_checkout_sha,
        None
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].state(now, now),
        CheckState::Pending
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].state(now, now + 40),
        CheckState::Failure
    );
    assert!(observed.evidence_current(&p, "abc", "def", now));
    assert!(!observed.evidence_current(&p, "new", "def", now));
    assert!(!observed.evidence_current(&p, "abc", "new", now));
    assert!(!observed.evidence_current(&p, "abc", "def", now + 60));
    assert!(!observed.evidence_current(&p, "abc", "def", now - 1));
    assert!(
        !f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .any(|s| s.contains("/statuses") || s.contains("/status?"))
    );
}
#[tokio::test]
async fn capability_changes_permissions_logs_and_workflows_block_without_writes() {
    let (f, mut p) = fixture().await;
    let now = github_service::now();
    p.delivery.as_mut().unwrap().actions.read_logs = true;
    grants(&f, &p);
    let mut c = f.client();
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("logs"))
    );
    f.data.lock().unwrap().redirects.insert(
        "/repos/owner/repo/actions/jobs/20/logs".into(),
        (302, format!("{}fixture-log", f.url)),
    );
    f.put("/fixture-log", json!("synthetic job log"));
    f.data.lock().unwrap().redirects.insert(
        "/repos/owner/repo/actions/jobs/21/logs".into(),
        (302, format!("{}fixture-log", f.url)),
    );
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .is_empty()
    );
    let mut changed = permissions(&p);
    changed["checks"] = json!("none");
    f.grant(changed);
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("permission"))
    );
    grants(&f, &p);
    f.put(
        "/repos/owner/repo/actions/workflows/8",
        json!({"path":".github/workflows/ci.yml","state":"disabled_manually"}),
    );
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("inactive"))
    );
    f.actions();
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E/protection",
        json!({}),
    );
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("protection"))
    );
    assert!(
        f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .all(|s| s.starts_with("GET ") || s.contains("/access_tokens"))
    );
    assert_eq!(permissions(&p)["actions"], "read");
    p.delivery.as_mut().unwrap().actions.rerun_actions = true;
    p.delivery.as_mut().unwrap().actions.rerequest_checks = true;
    assert_eq!(permissions(&p)["actions"], "write");
    assert_eq!(permissions(&p)["checks"], "write");
}
#[test]
fn legacy_migration_contract_gaps_and_test_merge_protection() {
    let legacy = serde_json::to_value(action_policy()).unwrap();
    assert!(legacy["delivery"].is_null());
    let mut without_delivery = legacy.clone();
    without_delivery.as_object_mut().unwrap().remove("delivery");
    let decoded: Policy = serde_json::from_value(without_delivery).unwrap();
    assert!(decoded.delivery.is_none());
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let mut p = policy_v1();
    assert!(p.delivery.as_ref().unwrap().blockers(&p).is_empty());
    let mut c = p.delivery.take().unwrap();
    c.schema_version = 2;
    c.pre_merge.wait_seconds = 0;
    c.pre_merge.checkout = PreMergeSource::TestMerge;
    c.protection = json!({});
    c.actions.merge = true;
    if let PostMerge::Checks { checks, .. } = &mut c.post_merge {
        checks[0].trigger = Trigger::PullRequest;
    }
    let blockers = c.blockers(&p);
    for expected in [
        "schema_version",
        "deadline",
        "test-merge",
        "PR-only",
        "merge method",
    ] {
        assert!(
            blockers.iter().any(|v| v.contains(expected)),
            "{blockers:?}"
        );
    }
    c = contract();
    c.protection["required_pull_request_reviews"] = json!({"required_approving_review_count":1});
    c.actions.merge = true;
    c.actions.merge_method = Some("squash".into());
    assert!(c.blockers(&p).iter().any(|v| v.contains("review")));
    c.post_merge = PostMerge::FixedValidation {
        plan_id: "".into(),
        configuration_sha256: "no".into(),
        authorization: "".into(),
        wait_seconds: 0,
    };
    assert!(
        c.blockers(&p)
            .iter()
            .any(|v| v.contains("fixed validation"))
    );
}
#[test]
fn test_merge_requires_strict_checks_and_admin_enforcement() {
    let p = policy_v1();
    let mut c = contract();
    c.pre_merge.source = PreMergeSource::TestMerge;
    assert!(c.blockers(&p).is_empty());
    for protection in [
        json!({"required_status_checks":{"strict":false,"checks":[{"context":"ci","app_id":42}]},"enforce_admins":{"enabled":true}}),
        json!({"required_status_checks":{"strict":true,"checks":[{"context":"ci","app_id":42}]},"enforce_admins":{"enabled":false}}),
        json!({"required_status_checks":{"strict":true,"checks":[]},"enforce_admins":{"enabled":true}}),
        json!({"required_status_checks":{"strict":true},"enforce_admins":{"enabled":true}}),
    ] {
        c.protection = protection;
        assert!(c.blockers(&p).iter().any(|v| v.contains("test-merge")));
    }
}

#[test]
fn external_checks_reject_missing_publishers_and_unpinned_triggers() {
    let p = policy_v1();
    for source in [
        Source::CheckRun { app_id: 0 },
        Source::Status { creator_id: 0 },
        Source::CheckRun { app_id: 42 },
        Source::Status { creator_id: 5 },
    ] {
        let mut c = contract();
        c.pre_merge.checks[0].selector.source = source;
        assert!(
            c.blockers(&p)
                .iter()
                .any(|v| v.contains("external publisher"))
        );
    }
}

#[test]
fn rerun_preserves_failures_and_conflicts_never_pick_a_green() {
    let p = action_policy();
    let mut first = run();
    first["run_attempt"] = json!(1);
    let checks = vec![check(1, "failure"), check(2, "success")];
    let resolved = resolve(&p.required[0], &checks, &[], &[run(), first], &[job()]);
    assert_eq!(resolved.state, CheckState::Success);
    assert_eq!(resolved.history.as_ref().unwrap().len(), 2);
    assert_eq!(
        resolved.history.as_ref().unwrap()[0]["conclusion"],
        "failure"
    );
    let mut conflicting = run();
    conflicting["id"] = json!(77);
    assert_eq!(
        resolve(
            &p.required[0],
            &checks,
            &[],
            &[run(), conflicting],
            &[job()]
        )
        .state,
        CheckState::Ambiguous
    );
    assert_eq!(
        resolve(&p.required[0], &checks, &[], &[run()], &[job(), job()]).state,
        CheckState::Ambiguous
    );
    let mut missing = run();
    missing["run_attempt"] = Value::Null;
    assert_eq!(
        resolve(&p.required[0], &checks, &[], &[missing], &[job()]).state,
        CheckState::Ambiguous
    );
    let status_selector = policy().required.remove(0);
    assert_eq!(
        resolve(
            &status_selector,
            &[],
            &[status(4, "success"), status(4, "failure")],
            &[],
            &[]
        )
        .state,
        CheckState::Ambiguous
    );
    for conclusion in ["skipped", "neutral", "cancelled", "N/A"] {
        assert_eq!(
            resolve(
                &p.required[0],
                &[check(2, conclusion)],
                &[],
                &[run()],
                &[job()]
            )
            .state,
            CheckState::Failure
        );
    }
}
#[tokio::test]
async fn ordered_store_preserves_originals_and_invalidates_policy_identity() {
    let pool = database().await;
    let (f, p) = fixture().await;
    let now = github_service::now();
    github_store::configure(&pool, &p, 1).await.unwrap();
    github_store::link(&pool, 99, 1, 1).await.unwrap();
    let fresh = github_observe::preflight(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    github_store::save_capability(&pool, &fresh).await.unwrap();
    let mut old = fresh.clone();
    old.checked_at = now - 1;
    old.blockers.push("old".into());
    github_store::save_capability(&pool, &old).await.unwrap();
    let saved: Value = sqlx::query_scalar("SELECT capability FROM github_repository")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved["blockers"], json!([]));
    let new = github_observe::observe(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    github_store::save_observation(&pool, &new).await.unwrap();
    let mut old = new.clone();
    old.last_synced_at = now - 1;
    old.head = "old".into();
    github_store::save_observation(&pool, &old).await.unwrap();
    let saved: Value = sqlx::query_scalar("SELECT observation FROM github_pr")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved["head"], "abc");
    old.last_synced_at = now;
    github_store::save_observation(&pool, &old).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT stale FROM github_pr")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    let mut conflict = fresh.clone();
    conflict.blockers.push("conflicting".into());
    github_store::save_capability(&pool, &conflict)
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT stale FROM github_repository")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM github_evidence_history").await,
        6
    );
    let mut p2 = p.clone();
    p2.wait_seconds += 1;
    assert!(!github_store::configure(&pool, &p2, 1).await.unwrap());
    p2.version += 1;
    sqlx::query("UPDATE repository SET version=2")
        .execute(&pool)
        .await
        .unwrap();
    github_store::configure(&pool, &p2, 1).await.unwrap();
    old.last_synced_at = now + 1;
    github_store::save_observation(&pool, &old).await.unwrap();
    let saved: Value = sqlx::query_scalar("SELECT observation FROM github_pr")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved["head"], "abc");
    assert!(!new.evidence_current(&p2, "abc", "def", now));
    pool.close().await;
}

#[tokio::test]
async fn external_status_and_check_sources_need_pinned_triggers_and_publishers() {
    for status_only in [true, false] {
        let (f, mut p) = fixture().await;
        let source = if status_only {
            Source::Status { creator_id: 5 }
        } else {
            Source::CheckRun { app_id: 42 }
        };
        p.required[0].source = source.clone();
        let contract = p.delivery.as_mut().unwrap();
        let pre = &mut contract.pre_merge.checks[0];
        pre.selector.source = source.clone();
        pre.job = None;
        pre.trigger = Trigger::External {
            path: "trigger.json".into(),
            blob_sha: "external-pin".into(),
            event: "pull_request".into(),
        };
        if let PostMerge::Checks { checks, .. } = &mut contract.post_merge {
            checks[0].selector.source = source;
            checks[0].job = None;
            checks[0].trigger = Trigger::External {
                path: "trigger.json".into(),
                blob_sha: "external-pin".into(),
                event: "push".into(),
            };
        }
        if status_only {
            contract.protection["required_status_checks"]["checks"][0]["app_id"] = Value::Null;
        }
        f.put(
            "/repos/owner/repo/branches/%6D%61%69%6E/protection",
            contract.protection.clone(),
        );
        grants(&f, &p);
        f.put(
            "/repos/owner/repo/contents/%74%72%69%67%67%65%72%2E%6A%73%6F%6E",
            json!({"sha":"external-pin"}),
        );
        f.put(
            "/repos/owner/repo/commits/merged/statuses",
            json!([status(9, "success")]),
        );
        f.put(
            "/repos/owner/repo/check-suites/9/check-runs",
            json!({"check_runs":[check(2,"success")]}),
        );
        f.put(
            "/repos/owner/repo/check-suites/9/check-runs?filter=all&per_page=100&page=2",
            json!({"check_runs":[]}),
        );
        let cap = github_observe::preflight(&mut f.client(), &p, 1, github_service::now())
            .await
            .unwrap();
        assert!(cap.blockers.is_empty(), "{:?}", cap.blockers);
        assert_eq!(
            cap.permissions["statuses"],
            if status_only {
                json!("read")
            } else {
                Value::Null
            }
        );
        p.delivery.as_mut().unwrap().actions.read_logs = true;
        let logs = github_observe::preflight(&mut f.client(), &p, 1, github_service::now())
            .await
            .unwrap();
        for expected in [
            "external publisher log adapter unavailable",
            "job identity missing",
        ] {
            assert!(
                logs.blockers.iter().any(|v| v.contains(expected)),
                "{:?}",
                logs.blockers
            );
        }
        assert_eq!(logs.configuration["logs"], json!([]));
        if status_only {
            let mut forged = status(99, "success");
            forged["creator"]["id"] = json!(999);
            f.put(
                "/repos/owner/repo/commits/abc/statuses",
                json!([forged, status(1, "failure")]),
            );
        } else {
            let mut forged = check(99, "success");
            forged["app"]["id"] = json!(999);
            f.put(
                "/repos/owner/repo/check-suites/9/check-runs",
                json!({"check_runs":[forged,check(1,"failure")]}),
            );
        }
        let observation = github_observe::observe(&mut f.client(), &p, 1, github_service::now())
            .await
            .unwrap();
        assert_eq!(observation.checks[0].state, CheckState::Failure);
    }
}
fn validation(
    candidate: &codexsymphony_server::validation::Candidate,
    trusted: &codexsymphony_server::validation::TrustedIdentity,
) -> codexsymphony_server::validation::ValidationEvidence {
    use codexsymphony_server::validation::*;
    ValidationEvidence {
        candidate: candidate.clone(),
        trusted: trusted.clone(),
        source_before: candidate.sha.clone(),
        source_after: candidate.sha.clone(),
        entry_before: trusted.protected_entry_sha256.clone(),
        entry_after: trusted.protected_entry_sha256.clone(),
        steps: vec![StepEvidence {
            id: "ci".into(),
            command: vec!["/gate-entry".into()],
            exit_code: Some(0),
            output: "passed".into(),
            output_sha256: sha256("passed"),
            log_ref: "synthetic/log".into(),
            consumer: "fixed-validation".into(),
            code_failure: false,
        }],
    }
}
#[tokio::test]
async fn fixed_validation_plan_and_checkout_are_independent_identity_evidence() {
    use codexsymphony_server::{
        validation::*,
        validation_runner::{Plan, Step},
    };
    let (f, mut p) = fixture().await;
    let entry = f.root.join("gate-entry");
    std::fs::write(&entry, "synthetic trusted entry").unwrap();
    let plan = Plan {
        entry,
        entry_sha256: sha256("synthetic trusted entry"),
        steps: vec![Step {
            id: "ci".into(),
            command: vec!["/gate-entry".into()],
            timeout_seconds: 10,
            code_failure: false,
        }],
    };
    let identity = plan.identity().unwrap();
    let path = f.root.join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    p.delivery.as_mut().unwrap().post_merge = PostMerge::FixedValidation {
        plan_id: path.to_str().unwrap().into(),
        configuration_sha256: identity.config_sha256.clone(),
        authorization: "reviewed repository policy v1".into(),
        wait_seconds: 20,
    };
    grants(&f, &p);
    let now = github_service::now();
    let cap = github_observe::preflight(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    assert!(cap.blockers.is_empty(), "{:?}", cap.blockers);
    let mut observation = github_observe::observe(&mut f.client(), &p, 2, now)
        .await
        .unwrap();
    let phase = &mut observation.phases.as_mut().unwrap()[1];
    assert_eq!(phase.state(now, now), CheckState::Pending);
    let candidate = Candidate {
        sha: "merged".into(),
        tree: "synthetic-tree".into(),
        immutable: true,
    };
    let evidence = validation(&candidate, &identity);
    let mut wrong = candidate.clone();
    wrong.sha = "abc".into();
    assert!(
        phase
            .bind_validation(evidence.clone(), &wrong, &identity, &["ci".into()])
            .is_err()
    );
    let mut bad = evidence.clone();
    bad.steps[0].exit_code = Some(1);
    assert!(
        phase
            .bind_validation(bad, &candidate, &identity, &["ci".into()])
            .is_err()
    );
    phase
        .bind_validation(evidence, &candidate, &identity, &["ci".into()])
        .unwrap();
    assert_eq!(phase.actual_checkout_sha.as_deref(), Some("merged"));
    assert_eq!(phase.state(now, now), CheckState::Success);
    let pre = &mut observation.phases.as_mut().unwrap()[0];
    let head = Candidate {
        sha: "abc".into(),
        ..candidate.clone()
    };
    pre.bind_validation(
        validation(&head, &identity),
        &head,
        &identity,
        &["ci".into()],
    )
    .unwrap();
    assert_eq!(pre.state(now, now), CheckState::Success);
    std::fs::write(&plan.entry, "changed").unwrap();
    assert!(
        github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("plan unavailable"))
    );
}
#[tokio::test]
async fn test_merge_unknown_merge_and_current_job_conflicts_fail_closed() {
    let (f, mut p) = fixture().await;
    p.delivery.as_mut().unwrap().pre_merge.checkout = PreMergeSource::TestMerge;
    let now = github_service::now();
    let observed = github_observe::observe(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    assert_eq!(
        observed.phases.as_ref().unwrap()[0].check_sha.as_deref(),
        Some("abc")
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[0]
            .expected_checkout_sha
            .as_deref(),
        Some("test-merge")
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[0].actual_checkout_sha,
        None
    );
    let mut unknown = pr();
    unknown.as_object_mut().unwrap().remove("merged");
    unknown["state"] = json!("closed");
    f.put("/repos/owner/repo/pulls/1", unknown);
    let observed = github_observe::observe(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    assert_eq!(observed.merge, MergeFact::Unknown);
    assert!(observed.merged_sha.is_none());
    assert_eq!(observed.phases.as_ref().unwrap().len(), 1);
    f.fail("/repos/owner/repo/pulls/1", vec![404]);
    assert_eq!(
        github_observe::observe(&mut f.client(), &p, 1, now)
            .await
            .unwrap_err()
            .status,
        Some(404)
    );
    let mut j = job();
    j["name"] = json!("wrong-job");
    f.put(
        "/repos/owner/repo/actions/runs/10/attempts/2/jobs",
        json!({"jobs":[j]}),
    );
    assert_eq!(
        github_observe::observe(&mut f.client(), &p, 1, now)
            .await
            .unwrap()
            .checks[0]
            .state,
        CheckState::Ambiguous
    );
}

#[tokio::test]
async fn changing_head_or_base_during_collection_rejects_the_entire_snapshot() {
    for side in ["head", "base"] {
        let (f, p) = fixture().await;
        f.data.lock().unwrap().delays.insert(
            "/repos/owner/repo/commits/abc/check-suites".into(),
            std::time::Duration::from_millis(100),
        );
        let mut client = f.client();
        let collect = tokio::spawn(async move {
            github_observe::observe(&mut client, &p, 1, github_service::now()).await
        });
        loop {
            if f.data
                .lock()
                .unwrap()
                .seen
                .iter()
                .any(|s| s.starts_with("GET /repos/owner/repo/pulls/1?"))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let mut changed = pr();
        changed[side]["sha"] = json!("raced-sha");
        f.put("/repos/owner/repo/pulls/1", changed);
        assert_eq!(
            collect.await.unwrap().unwrap_err().code,
            "github_identity_conflict"
        );
    }
}

#[tokio::test]
async fn workflow_blob_is_checked_on_every_observed_source_not_only_capability_probe() {
    let (f, p) = fixture().await;
    let now = github_service::now();
    assert!(
        github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .unwrap()
            .blockers
            .is_empty()
    );
    f.put("/repos/owner/repo/contents/%2E%67%69%74%68%75%62%2F%77%6F%72%6B%66%6C%6F%77%73%2F%63%69%2E%79%6D%6C?ref=%6D%65%72%67%65%64",json!({"sha":"weakened"}));
    let observed = github_observe::observe(&mut f.client(), &p, 2, now)
        .await
        .unwrap();
    assert_eq!(
        observed.phases.as_ref().unwrap()[0].checks[0].state,
        CheckState::Success
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].checks[0].state,
        CheckState::Ambiguous
    );
    assert_eq!(
        observed.phases.as_ref().unwrap()[1].checks[0].evidence[0]["configuration"]["head_blob"],
        "weakened"
    );
}

#[tokio::test]
async fn selected_merge_and_dispatch_capabilities_do_not_mutate_remote_rules() {
    let (f, mut p) = fixture().await;
    let now = github_service::now();
    for method in ["merge", "squash", "rebase"] {
        let contract = p.delivery.as_mut().unwrap();
        contract.actions.merge = true;
        contract.actions.merge_method = Some(method.into());
        f.put("/repos/owner/repo",json!({"id":99,"full_name":"owner/repo","default_branch":"main","archived":false,"allow_merge_commit":true,"allow_squash_merge":true,"allow_rebase_merge":true}));
        assert!(
            github_observe::preflight(&mut f.client(), &p, 1, now)
                .await
                .unwrap()
                .blockers
                .is_empty()
        );
    }
    f.put(
        "/repos/owner/repo",
        json!({"id":99,"full_name":"owner/repo","default_branch":"main","archived":false}),
    );
    assert!(
        github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .unwrap()
            .blockers
            .iter()
            .any(|v| v.contains("merge method unavailable"))
    );
    let contract = p.delivery.as_mut().unwrap();
    contract.actions.merge = false;
    if let PostMerge::Checks { checks, .. } = &mut contract.post_merge {
        checks[0].trigger = Trigger::WorkflowDispatch;
        if let Source::Actions { event, .. } = &mut checks[0].selector.source {
            *event = "workflow_dispatch".into();
        }
    }
    {
        let mut data = f.data.lock().unwrap();
        data.routes
            .get_mut("/repos/owner/repo/actions/runs?head_sha=merged&per_page=100&page=1")
            .unwrap()["workflow_runs"][0]["event"] = json!("workflow_dispatch");
    }
    grants(&f, &p);
    let capability = github_observe::preflight(&mut f.client(), &p, 1, now)
        .await
        .unwrap();
    assert!(capability.blockers.is_empty(), "{:?}", capability.blockers);
    assert_eq!(capability.permissions["actions"], "write");
    let observed = github_observe::observe(&mut f.client(), &p, 2, now)
        .await
        .unwrap();
    assert_eq!(observed.test_merge_sha, None);
    assert_eq!(observed.merged_sha.as_deref(), Some("merged"));
    assert!(
        f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .all(|s| s.starts_with("GET ") || s.contains("/access_tokens"))
    );
}
#[test]
fn incomplete_authorization_and_unsupported_protection_remain_blockers() {
    let p = policy_v1();
    let mut c = contract();
    c.actions.merge = true;
    c.actions.merge_method = Some("merge".into());
    c.protection["required_signatures"] = json!({"enabled":true});
    c.rules = vec![
        json!({"type":"required_deployments"}),
        json!({"type":"pull_request","parameters":{"require_code_owner_reviews":true}}),
    ];
    let blockers = c.blockers(&p);
    for expected in [
        "signed-commit",
        "unsupported merge constraint",
        "independent approvals",
    ] {
        assert!(
            blockers.iter().any(|v| v.contains(expected)),
            "{blockers:?}"
        );
    }
    let pre = &mut c.pre_merge.checks[0];
    pre.applicability = "conditional".into();
    pre.selector.name.clear();
    pre.job = None;
    if let Source::Actions { app_id, .. } = &mut pre.selector.source {
        *app_id = 0;
    }
    pre.trigger = Trigger::External {
        path: "".into(),
        blob_sha: "".into(),
        event: "other".into(),
    };
    assert!(c.blockers(&p).len() >= 6);
    c.pre_merge.checks.clear();
    assert!(
        c.blockers(&p)
            .iter()
            .any(|v| v.contains("checks and deadline"))
    );
    c.post_merge = PostMerge::Checks {
        checks: Vec::new(),
        wait_seconds: 0,
        probe_pr: 0,
    };
    assert!(
        c.blockers(&p)
            .iter()
            .any(|v| v.contains("post_merge: checks"))
    );
}

#[tokio::test]
async fn log_download_never_forwards_credentials_and_rejects_untrusted_or_unbounded_content() {
    let (f, p) = fixture().await;
    let now = github_service::now();
    for location in [
        "https://untrusted.invalid/log",
        "https://fixture@untrusted.invalid/log",
        "not a URL",
    ] {
        f.data.lock().unwrap().redirects.insert(
            "/repos/owner/repo/actions/jobs/20/logs".into(),
            (302, location.into()),
        );
        assert!(f.client().logs_readable(&p, 20, now).await.is_err());
    }
    f.data.lock().unwrap().redirects.insert(
        "/repos/owner/repo/actions/jobs/20/logs".into(),
        (302, format!("{}fixture-log", f.url)),
    );
    f.put("/fixture-log", json!("safe synthetic log"));
    let proof = f.client().logs_readable(&p, 20, now).await.unwrap();
    assert_eq!(proof["job_id"], 20);
    assert_eq!(proof["sha256"].as_str().unwrap().len(), 64);
    f.fail("/fixture-log", vec![204, 500]);
    assert!(f.client().logs_readable(&p, 20, now).await.is_err());
    assert!(f.client().logs_readable(&p, 20, now).await.is_err());
    f.put("/fixture-log", json!("x".repeat(9 * 1024 * 1024)));
    assert!(f.client().logs_readable(&p, 20, now).await.is_err());
}
