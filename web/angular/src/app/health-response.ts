// harness-contract-sha256: feb49ee0737abccd74864c3e43f01891669139a491636babddffe401eaef341f
export interface HealthResponse {
  database: 'ok' | 'unavailable';
  status: 'ok' | 'unavailable';
}
export interface GetRepositoryResponse {
  deployment_network: string[];
  network_status: string;
  repositories: {
    repository: {
      base_branch: string;
      environment?: string | null;
      github_repository_id: number;
      hooks?: {
        argv: string[];
        event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
        name: string;
        output_limit_bytes: number;
        replay?: 'idempotent' | 'never' | 'reconcile';
        roles: ('coding' | 'repair' | 'validation')[];
        script_identity: string;
        timeout_seconds: number;
      }[];
      model?: string | null;
      policy: {
        allowed_checks: string[];
        gate_recovery_policy: string;
        max_timeout_seconds: number;
        model_work_seconds: number;
        token_limit: number;
        turn_limit: number;
      };
      project: string;
      reason: string;
      remote: string;
      revoked: boolean;
    };
    version: number;
  }[];
  repository_ready: boolean;
  runtime_ready: boolean;
}
export type ConfigureRepositoryResponse =
  | {
      repository: {
        base_branch: string;
        environment?: string | null;
        github_repository_id: number;
        hooks?: {
          argv: string[];
          event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
          name: string;
          output_limit_bytes: number;
          replay?: 'idempotent' | 'never' | 'reconcile';
          roles: ('coding' | 'repair' | 'validation')[];
          script_identity: string;
          timeout_seconds: number;
        }[];
        model?: string | null;
        policy: {
          allowed_checks: string[];
          gate_recovery_policy: string;
          max_timeout_seconds: number;
          model_work_seconds: number;
          token_limit: number;
          turn_limit: number;
        };
        project: string;
        reason: string;
        remote: string;
        revoked: boolean;
      };
      version: number;
    }
  | { error: string };
export interface ConfigureRepositoryRequest {
  repository: {
    base_branch: string;
    environment?: string | null;
    github_repository_id: number;
    hooks?: {
      argv: string[];
      event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
      name: string;
      output_limit_bytes: number;
      replay?: 'idempotent' | 'never' | 'reconcile';
      roles: ('coding' | 'repair' | 'validation')[];
      script_identity: string;
      timeout_seconds: number;
    }[];
    model?: string | null;
    policy: {
      allowed_checks: string[];
      gate_recovery_policy: string;
      max_timeout_seconds: number;
      model_work_seconds: number;
      token_limit: number;
      turn_limit: number;
    };
    project: string;
    reason: string;
    remote: string;
    revoked: boolean;
  };
  request_id: string;
  version: number;
}
export interface ListRequirementsResponse {
  requirements: {
    id: number;
    revision: number;
    state: string;
    title: string;
    version: number;
  }[];
}
export type CreateRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface CreateRequirementRequest {
  contract: {
    acceptance_criteria: { description: string; verification_ref: string }[];
    description: string;
    network_access: string[];
    title: string;
    validation_plan: {
      check: string;
      expected_result: string;
      id: string;
      selector: string;
      timeout_seconds: number;
    }[];
  };
  request_id: string;
  version: number;
}
export type GetRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export type UpdateRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface UpdateRequirementRequest {
  contract: {
    acceptance_criteria: { description: string; verification_ref: string }[];
    description: string;
    network_access: string[];
    title: string;
    validation_plan: {
      check: string;
      expected_result: string;
      id: string;
      selector: string;
      timeout_seconds: number;
    }[];
  };
  request_id: string;
  version: number;
}
export type ReadyRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface ReadyRequirementRequest {
  repository_version: number;
  request_id: string;
  version: number;
}
export type WithdrawRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface WithdrawRequirementRequest {
  repository_version: number;
  request_id: string;
  version: number;
}
export interface ExecutionStatusResponse {
  coding_blocker: string;
  coding_ready: false;
  paused: boolean;
  recovery_complete: boolean;
  requirement_id: number | null;
}
export type PauseExecutionResponse = { paused: true } | { error: string };
export interface PauseExecutionRequest { pause: true }
export type PauseRequirementResponse = { paused: true } | { error: string };
export interface PauseRequirementRequest { pause: true }
export type GetOperationsResponse =
  | {
      environments?: { observed_at: string; report: string; stage: string }[];
      events: { created_at: string; kind: string; version: number }[];
      external: {
        head_sha: string;
        observation: string | null;
        pr_number: number | null;
        repository: string;
        revision: number;
        stale: boolean;
      }[];
      materials: {
        channel: string;
        discarded_bytes: number;
        kept_bytes: number;
        run_id: string;
        status: string;
      }[];
      metrics: {
        build_test_coverage_subphases?: { status: 'not_applicable' | 'unknown' };
        cache_hit_rate?: { status: 'not_applicable' | 'unknown' };
        cached: number | null;
        ci_wait?: {
          seconds?: number;
          source?: string;
          status: 'known' | 'not_applicable' | 'unknown';
        };
        environment_samples?: { elapsed_ms: number; source: string; stage: string }[];
        human_seconds: number | null;
        input: number | null;
        interventions: number;
        model_calls: number;
        output: number | null;
        phases: { complete: boolean; phase: string; seconds: number }[];
        reasons: string[];
        repair_count: number;
        stage_samples?: {
          complete: boolean;
          phase: string;
          seconds: number | null;
          source: string;
        }[];
        to_pr_seconds: number | null;
        zero_intervention: { denominator: number; numerator: number; phase: string };
      };
      preparation: {
        attempts: number;
        code: string | null;
        detail: string | null;
        phase: string;
        run_id: string;
        todo: boolean;
      }[];
      questions: {
        answered: boolean;
        id: string;
        questions: { id: string; options: string[]; question: string }[];
        resume_state: string;
        revision: number;
        run_id: string;
        version: number;
      }[];
      recoveries?: {
        attempts: number;
        candidate_sha: string;
        decision: string;
        event_key: string;
        log_ref: string;
        next_attempt_at: number | null;
        phase: string;
        reason: string;
      }[];
      requirement: {
        cancel_requested: boolean;
        cleanup_complete: boolean;
        id: number;
        paused: boolean;
        revision: number;
        state: string;
        version: number;
      };
      runs: {
        blocker: string | null;
        created_at: string;
        id: string;
        phase: string;
        quiescent: boolean;
        revision: number;
        state: string;
        waiting: string;
      }[];
      storage: { blocked: boolean; error: string | null };
      storage_lifecycle: string;
      storage_usage?: {
        actual_bytes: number | null;
        available_bytes: number | null;
        categories: {
          actual_bytes: number | null;
          limit_bytes: number;
          name: string;
          reserved_bytes: number;
          retention_seconds: number;
        }[];
        classification_todo: number;
        cleanup_todo: number;
        configured: boolean;
        control_bytes: number | null;
        global_limit: number | null;
        materials: {
          actual_bytes: number;
          attempts: number;
          category: string;
          deleted_at: number | null;
          expires_at: number;
          failure: string | null;
          id: string;
          identity: string;
          kind: string;
          next_attempt_at: number | null;
          protection: string | null;
          reason: string | null;
          replacement: string | null;
          resolved_by: string | null;
          run_id: string;
          status: string;
          summary: string;
        }[];
        measured_at: number | null;
        policy_version: string | null;
        protected_bytes: number;
        requirement_allocated: number;
        reserved_bytes: number;
      };
      validations: {
        candidate_sha: string;
        failure: string | null;
        id: string;
        result: string;
        revision: number;
        source_run_id: string;
        stage: string;
      }[];
    }
  | { error: string };
export type ControlOperationsResponse = { version: number } | { error: string };
export interface ControlOperationsRequest {
  action: 'cancel' | 'delivery_recheck' | 'pause' | 'recheck' | 'resume' | 'storage_recheck';
  request_id: string;
  version: number;
}
export type GetEvidenceResponse = { preview_only: boolean; text: string } | { error: string };
export interface GetInboxResponse { requirement_ids: number[] }
export type AnswerOperatorQuestionResponse = { saved: boolean } | { error: string };
export interface AnswerOperatorQuestionRequest {
  answers: { id: string; text: string }[];
  version: number;
}
export interface MultiGetRepositoryResponse {
  deployment_network: string[];
  network_status: string;
  repositories: {
    capability_blockers?: string[];
    capability_checked_at?: number | null;
    capability_error?: string | null;
    capability_http_status?: number | null;
    capability_stale?: boolean;
    delivery_ready?: boolean;
    id?: number;
    repository: {
      base_branch: string;
      environment?: string | null;
      github_repository_id: number;
      hooks?: {
        argv: string[];
        event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
        name: string;
        output_limit_bytes: number;
        replay?: 'idempotent' | 'never' | 'reconcile';
        roles: ('coding' | 'repair' | 'validation')[];
        script_identity: string;
        timeout_seconds: number;
      }[];
      model?: string | null;
      policy: {
        allowed_checks: string[];
        gate_recovery_policy: string;
        max_timeout_seconds: number;
        model_work_seconds: number;
        token_limit: number;
        turn_limit: number;
      };
      project: string;
      reason: string;
      remote: string;
      revoked: boolean;
    };
    version: number;
  }[];
  repository_ready: boolean;
  runtime_ready: boolean;
}
export type MultiConfigureRepositoryResponse =
  | {
      id?: number;
      repository: {
        base_branch: string;
        environment?: string | null;
        github_repository_id: number;
        hooks?: {
          argv: string[];
          event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
          name: string;
          output_limit_bytes: number;
          replay?: 'idempotent' | 'never' | 'reconcile';
          roles: ('coding' | 'repair' | 'validation')[];
          script_identity: string;
          timeout_seconds: number;
        }[];
        model?: string | null;
        policy: {
          allowed_checks: string[];
          gate_recovery_policy: string;
          max_timeout_seconds: number;
          model_work_seconds: number;
          token_limit: number;
          turn_limit: number;
        };
        project: string;
        reason: string;
        remote: string;
        revoked: boolean;
      };
      version: number;
    }
  | { error: string };
export interface MultiConfigureRepositoryRequest {
  repository: {
    base_branch: string;
    environment?: string | null;
    github_repository_id: number;
    hooks?: {
      argv: string[];
      event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
      name: string;
      output_limit_bytes: number;
      replay?: 'idempotent' | 'never' | 'reconcile';
      roles: ('coding' | 'repair' | 'validation')[];
      script_identity: string;
      timeout_seconds: number;
    }[];
    model?: string | null;
    policy: {
      allowed_checks: string[];
      gate_recovery_policy: string;
      max_timeout_seconds: number;
      model_work_seconds: number;
      token_limit: number;
      turn_limit: number;
    };
    project: string;
    reason: string;
    remote: string;
    revoked: boolean;
  };
  repository_id?: number;
  request_id: string;
  version: number;
}
export interface MultiListRequirementsResponse {
  requirements: {
    id: number;
    repository_id?: number;
    revision: number;
    state: string;
    title: string;
    version: number;
  }[];
}
export type MultiCreateRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      repository_id?: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_id?: number;
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface MultiCreateRequirementRequest {
  contract: {
    acceptance_criteria: { description: string; verification_ref: string }[];
    description: string;
    network_access: string[];
    title: string;
    validation_plan: {
      check: string;
      expected_result: string;
      id: string;
      selector: string;
      timeout_seconds: number;
    }[];
  };
  repository_id?: number;
  request_id: string;
  version: number;
}
export type MultiGetRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      repository_id?: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_id?: number;
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export type MultiUpdateRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      repository_id?: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_id?: number;
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface MultiUpdateRequirementRequest {
  contract: {
    acceptance_criteria: { description: string; verification_ref: string }[];
    description: string;
    network_access: string[];
    title: string;
    validation_plan: {
      check: string;
      expected_result: string;
      id: string;
      selector: string;
      timeout_seconds: number;
    }[];
  };
  repository_id?: number;
  request_id: string;
  version: number;
}
export type MultiReadyRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      repository_id?: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_id?: number;
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface MultiReadyRequirementRequest {
  repository_version: number;
  request_id: string;
  version: number;
}
export type MultiWithdrawRequirementResponse =
  | {
      authorization_valid: boolean;
      contract: {
        acceptance_criteria: { description: string; verification_ref: string }[];
        description: string;
        network_access: string[];
        title: string;
        validation_plan: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        }[];
      };
      created_at: string;
      creator: string;
      id: number;
      repository_id?: number;
      revision: number;
      snapshots: {
        ac_ids: string[];
        contract: {
          acceptance_criteria: { description: string; verification_ref: string }[];
          description: string;
          network_access: string[];
          title: string;
          validation_plan: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          }[];
        };
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        repository_id?: number;
        repository_version: number;
        reviewer: string;
        revision: number;
      }[];
      state: string;
      version: number;
    }
  | { error: string };
export interface MultiWithdrawRequirementRequest {
  repository_version: number;
  request_id: string;
  version: number;
}
export interface ListDraftsResponse {
  drafts: { goal: string; id: string; state: string; version: number }[];
}
export type ImportDraftResponse =
  | {
      document: {
        children: {
          acceptance_criteria: { description: string; id: string }[];
          depends_on: string[];
          goal: string;
          id: string;
          kind: 'code_change' | 'validation_only';
          order: number;
          parent_id: string;
          repository_id: number | null;
          validation_plan: string;
        }[];
        parent: {
          acceptance_criteria: { description: string; id: string }[];
          goal: string;
          id: string;
          scope: string;
        };
        schema: string;
      };
      id: string;
      source: { format: 'json' | 'markdown'; label: string; text: string };
      source_sha256: string;
      state: 'Draft';
      version: number;
      warnings: string[];
    }
  | { error: string };
export interface ImportDraftRequest {
  source: { format: 'json' | 'markdown'; label: string; text: string };
  version: number;
}
export type GetDraftResponse =
  | {
      document: {
        children: {
          acceptance_criteria: { description: string; id: string }[];
          depends_on: string[];
          goal: string;
          id: string;
          kind: 'code_change' | 'validation_only';
          order: number;
          parent_id: string;
          repository_id: number | null;
          validation_plan: string;
        }[];
        parent: {
          acceptance_criteria: { description: string; id: string }[];
          goal: string;
          id: string;
          scope: string;
        };
        schema: string;
      };
      id: string;
      source: { format: 'json' | 'markdown'; label: string; text: string };
      source_sha256: string;
      state: 'Draft';
      version: number;
      warnings: string[];
    }
  | { error: string };
export type UpdateDraftResponse =
  | {
      document: {
        children: {
          acceptance_criteria: { description: string; id: string }[];
          depends_on: string[];
          goal: string;
          id: string;
          kind: 'code_change' | 'validation_only';
          order: number;
          parent_id: string;
          repository_id: number | null;
          validation_plan: string;
        }[];
        parent: {
          acceptance_criteria: { description: string; id: string }[];
          goal: string;
          id: string;
          scope: string;
        };
        schema: string;
      };
      id: string;
      source: { format: 'json' | 'markdown'; label: string; text: string };
      source_sha256: string;
      state: 'Draft';
      version: number;
      warnings: string[];
    }
  | { error: string };
export interface UpdateDraftRequest {
  source: { format: 'json' | 'markdown'; label: string; text: string };
  version: number;
}
export interface ListGenerationsResponse {
  generations: {
    completed_at: string | null;
    created_at: string;
    draft_id: string;
    error: string | null;
    evidence: {
      model?: string;
      reserved_turns?: number;
      runtime_user_agent?: string;
      settings_sha256?: string;
      thread_id?: string;
      turn_id?: string;
    };
    fingerprint: string;
    id: string;
    input_version: number;
    limits: { model_seconds: number; tokens: number; turns: number };
    output: string | null;
    output_version: number | null;
    request: {
      draft_id: string | null;
      label: string;
      request_id: string;
      text: string;
      version: number;
    };
    status: 'conflict' | 'failed' | 'interrupted' | 'running' | 'succeeded';
    usage: {
      cached: number | null;
      complete: boolean;
      input: number | null;
      model_seconds: number | null;
      output: number | null;
    };
  }[];
}
export type GenerateDraftResponse =
  | {
      completed_at: string | null;
      created_at: string;
      draft_id: string;
      error: string | null;
      evidence: {
        model?: string;
        reserved_turns?: number;
        runtime_user_agent?: string;
        settings_sha256?: string;
        thread_id?: string;
        turn_id?: string;
      };
      fingerprint: string;
      id: string;
      input_version: number;
      limits: { model_seconds: number; tokens: number; turns: number };
      output: string | null;
      output_version: number | null;
      request: {
        draft_id: string | null;
        label: string;
        request_id: string;
        text: string;
        version: number;
      };
      status: 'conflict' | 'failed' | 'interrupted' | 'running' | 'succeeded';
      usage: {
        cached: number | null;
        complete: boolean;
        input: number | null;
        model_seconds: number | null;
        output: number | null;
      };
    }
  | { error: string };
export interface GenerateDraftRequest {
  draft_id: string | null;
  label: string;
  request_id: string;
  text: string;
  version: number;
}
export type GetGenerationResponse =
  | {
      completed_at: string | null;
      created_at: string;
      draft_id: string;
      error: string | null;
      evidence: {
        model?: string;
        reserved_turns?: number;
        runtime_user_agent?: string;
        settings_sha256?: string;
        thread_id?: string;
        turn_id?: string;
      };
      fingerprint: string;
      id: string;
      input_version: number;
      limits: { model_seconds: number; tokens: number; turns: number };
      output: string | null;
      output_version: number | null;
      request: {
        draft_id: string | null;
        label: string;
        request_id: string;
        text: string;
        version: number;
      };
      status: 'conflict' | 'failed' | 'interrupted' | 'running' | 'succeeded';
      usage: {
        cached: number | null;
        complete: boolean;
        input: number | null;
        model_seconds: number | null;
        output: number | null;
      };
    }
  | { error: string };
export type GetGroupReviewResponse =
  | {
      authorizations: {
        id: number;
        snapshot: {
          affected?: string[];
          business_complete: boolean;
          document: {
            children: {
              acceptance_criteria: { description: string; id: string }[];
              depends_on: string[];
              goal: string;
              id: string;
              kind: 'code_change' | 'validation_only';
              order: number;
              parent_id: string;
              repository_id: number | null;
              validation_plan: string;
            }[];
            parent: {
              acceptance_criteria: { description: string; id: string }[];
              goal: string;
              id: string;
              scope: string;
            };
            schema: string;
          };
          group_budget: { model_seconds: number; tokens: number; turns: number };
          parent_revision: number;
          repositories: {
            id: number;
            repository: {
              base_branch: string;
              environment?: string | null;
              github_repository_id: number;
              hooks?: {
                argv: string[];
                event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
                name: string;
                output_limit_bytes: number;
                replay?: 'idempotent' | 'never' | 'reconcile';
                roles: ('coding' | 'repair' | 'validation')[];
                script_identity: string;
                timeout_seconds: number;
              }[];
              model?: string | null;
              policy: {
                allowed_checks: string[];
                gate_recovery_policy: string;
                max_timeout_seconds: number;
                model_work_seconds: number;
                token_limit: number;
                turn_limit: number;
              };
              project: string;
              reason: string;
              remote: string;
              revoked: boolean;
            };
            version: number;
          }[];
          review: {
            coverage: {
              child_ac: string;
              child_id: string;
              child_revision: number;
              parent_ac: string;
              step_id: string;
            }[];
            full_chain_acs: string[];
            group_budget: { model_seconds: number; tokens: number; turns: number } | null;
            items: {
              budget: { model_seconds: number; tokens: number; turns: number };
              child_id: string;
              integration?: {
                configuration_sha256: string;
                repositories: {
                  repair_scope: string;
                  repository_id: number;
                  repository_version: number;
                  selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
                }[];
              } | null;
              merged_baseline_review: string;
              repair_scope: string;
              repository_version: number;
              revision: number;
              verification: {
                ac_id: string;
                step: {
                  check: string;
                  expected_result: string;
                  id: string;
                  selector: string;
                  timeout_seconds: number;
                };
              }[];
            }[];
            parent_revision: number;
            semantic_review: string;
          };
          review_version: number;
          reviewer: string;
          scheduler_available: boolean;
        };
      }[];
      budgets: {
        item_id: string;
        limits: { model_seconds: number; tokens: number; turns: number };
        reserved: { model_seconds: number; tokens: number; turns: number };
        used: { model_seconds: number; tokens: number; turns: number };
      }[];
      business_complete: boolean;
      document: {
        children: {
          acceptance_criteria: { description: string; id: string }[];
          depends_on: string[];
          goal: string;
          id: string;
          kind: 'code_change' | 'validation_only';
          order: number;
          parent_id: string;
          repository_id: number | null;
          validation_plan: string;
        }[];
        parent: {
          acceptance_criteria: { description: string; id: string }[];
          goal: string;
          id: string;
          scope: string;
        };
        schema: string;
      };
      draft_id: string;
      draft_revision: number;
      execution?: {
        completed: number;
        items: {
          child_id: string;
          complete: boolean;
          depends_on: string[];
          kind: string;
          order: number;
          owner: boolean;
          repository_id: number | null;
          requirement_id: number | null;
          state: string;
          waiting_reason: string;
        }[];
        owner: number | null;
        parent_state: string;
        paused: boolean;
        total: number;
      };
      pending_edit?: {
        affected: string[];
        document: {
          children: {
            acceptance_criteria: { description: string; id: string }[];
            depends_on: string[];
            goal: string;
            id: string;
            kind: 'code_change' | 'validation_only';
            order: number;
            parent_id: string;
            repository_id: number | null;
            validation_plan: string;
          }[];
          parent: {
            acceptance_criteria: { description: string; id: string }[];
            goal: string;
            id: string;
            scope: string;
          };
          schema: string;
        };
        repositories: {
          id: number;
          repository: {
            base_branch: string;
            environment?: string | null;
            github_repository_id: number;
            hooks?: {
              argv: string[];
              event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
              name: string;
              output_limit_bytes: number;
              replay?: 'idempotent' | 'never' | 'reconcile';
              roles: ('coding' | 'repair' | 'validation')[];
              script_identity: string;
              timeout_seconds: number;
            }[];
            model?: string | null;
            policy: {
              allowed_checks: string[];
              gate_recovery_policy: string;
              max_timeout_seconds: number;
              model_work_seconds: number;
              token_limit: number;
              turn_limit: number;
            };
            project: string;
            reason: string;
            remote: string;
            revoked: boolean;
          };
          version: number;
        }[];
        review: {
          coverage: {
            child_ac: string;
            child_id: string;
            child_revision: number;
            parent_ac: string;
            step_id: string;
          }[];
          full_chain_acs: string[];
          group_budget: { model_seconds: number; tokens: number; turns: number } | null;
          items: {
            budget: { model_seconds: number; tokens: number; turns: number };
            child_id: string;
            integration?: {
              configuration_sha256: string;
              repositories: {
                repair_scope: string;
                repository_id: number;
                repository_version: number;
                selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
              }[];
            } | null;
            merged_baseline_review: string;
            repair_scope: string;
            repository_version: number;
            revision: number;
            verification: {
              ac_id: string;
              step: {
                check: string;
                expected_result: string;
                id: string;
                selector: string;
                timeout_seconds: number;
              };
            }[];
          }[];
          parent_revision: number;
          semantic_review: string;
        };
        version: number;
      } | null;
      queue: { authorization_id: number; state: string; version: number } | null;
      repositories: {
        id: number;
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        version: number;
      }[];
      review: {
        coverage: {
          child_ac: string;
          child_id: string;
          child_revision: number;
          parent_ac: string;
          step_id: string;
        }[];
        full_chain_acs: string[];
        group_budget: { model_seconds: number; tokens: number; turns: number } | null;
        items: {
          budget: { model_seconds: number; tokens: number; turns: number };
          child_id: string;
          integration?: {
            configuration_sha256: string;
            repositories: {
              repair_scope: string;
              repository_id: number;
              repository_version: number;
              selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
            }[];
          } | null;
          merged_baseline_review: string;
          repair_scope: string;
          repository_version: number;
          revision: number;
          verification: {
            ac_id: string;
            step: {
              check: string;
              expected_result: string;
              id: string;
              selector: string;
              timeout_seconds: number;
            };
          }[];
        }[];
        parent_revision: number;
        semantic_review: string;
      } | null;
      scheduler_available: boolean;
      version: number;
    }
  | { error: string };
export type SaveGroupReviewResponse =
  | {
      authorizations: {
        id: number;
        snapshot: {
          affected?: string[];
          business_complete: boolean;
          document: {
            children: {
              acceptance_criteria: { description: string; id: string }[];
              depends_on: string[];
              goal: string;
              id: string;
              kind: 'code_change' | 'validation_only';
              order: number;
              parent_id: string;
              repository_id: number | null;
              validation_plan: string;
            }[];
            parent: {
              acceptance_criteria: { description: string; id: string }[];
              goal: string;
              id: string;
              scope: string;
            };
            schema: string;
          };
          group_budget: { model_seconds: number; tokens: number; turns: number };
          parent_revision: number;
          repositories: {
            id: number;
            repository: {
              base_branch: string;
              environment?: string | null;
              github_repository_id: number;
              hooks?: {
                argv: string[];
                event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
                name: string;
                output_limit_bytes: number;
                replay?: 'idempotent' | 'never' | 'reconcile';
                roles: ('coding' | 'repair' | 'validation')[];
                script_identity: string;
                timeout_seconds: number;
              }[];
              model?: string | null;
              policy: {
                allowed_checks: string[];
                gate_recovery_policy: string;
                max_timeout_seconds: number;
                model_work_seconds: number;
                token_limit: number;
                turn_limit: number;
              };
              project: string;
              reason: string;
              remote: string;
              revoked: boolean;
            };
            version: number;
          }[];
          review: {
            coverage: {
              child_ac: string;
              child_id: string;
              child_revision: number;
              parent_ac: string;
              step_id: string;
            }[];
            full_chain_acs: string[];
            group_budget: { model_seconds: number; tokens: number; turns: number } | null;
            items: {
              budget: { model_seconds: number; tokens: number; turns: number };
              child_id: string;
              integration?: {
                configuration_sha256: string;
                repositories: {
                  repair_scope: string;
                  repository_id: number;
                  repository_version: number;
                  selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
                }[];
              } | null;
              merged_baseline_review: string;
              repair_scope: string;
              repository_version: number;
              revision: number;
              verification: {
                ac_id: string;
                step: {
                  check: string;
                  expected_result: string;
                  id: string;
                  selector: string;
                  timeout_seconds: number;
                };
              }[];
            }[];
            parent_revision: number;
            semantic_review: string;
          };
          review_version: number;
          reviewer: string;
          scheduler_available: boolean;
        };
      }[];
      budgets: {
        item_id: string;
        limits: { model_seconds: number; tokens: number; turns: number };
        reserved: { model_seconds: number; tokens: number; turns: number };
        used: { model_seconds: number; tokens: number; turns: number };
      }[];
      business_complete: boolean;
      document: {
        children: {
          acceptance_criteria: { description: string; id: string }[];
          depends_on: string[];
          goal: string;
          id: string;
          kind: 'code_change' | 'validation_only';
          order: number;
          parent_id: string;
          repository_id: number | null;
          validation_plan: string;
        }[];
        parent: {
          acceptance_criteria: { description: string; id: string }[];
          goal: string;
          id: string;
          scope: string;
        };
        schema: string;
      };
      draft_id: string;
      draft_revision: number;
      execution?: {
        completed: number;
        items: {
          child_id: string;
          complete: boolean;
          depends_on: string[];
          kind: string;
          order: number;
          owner: boolean;
          repository_id: number | null;
          requirement_id: number | null;
          state: string;
          waiting_reason: string;
        }[];
        owner: number | null;
        parent_state: string;
        paused: boolean;
        total: number;
      };
      pending_edit?: {
        affected: string[];
        document: {
          children: {
            acceptance_criteria: { description: string; id: string }[];
            depends_on: string[];
            goal: string;
            id: string;
            kind: 'code_change' | 'validation_only';
            order: number;
            parent_id: string;
            repository_id: number | null;
            validation_plan: string;
          }[];
          parent: {
            acceptance_criteria: { description: string; id: string }[];
            goal: string;
            id: string;
            scope: string;
          };
          schema: string;
        };
        repositories: {
          id: number;
          repository: {
            base_branch: string;
            environment?: string | null;
            github_repository_id: number;
            hooks?: {
              argv: string[];
              event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
              name: string;
              output_limit_bytes: number;
              replay?: 'idempotent' | 'never' | 'reconcile';
              roles: ('coding' | 'repair' | 'validation')[];
              script_identity: string;
              timeout_seconds: number;
            }[];
            model?: string | null;
            policy: {
              allowed_checks: string[];
              gate_recovery_policy: string;
              max_timeout_seconds: number;
              model_work_seconds: number;
              token_limit: number;
              turn_limit: number;
            };
            project: string;
            reason: string;
            remote: string;
            revoked: boolean;
          };
          version: number;
        }[];
        review: {
          coverage: {
            child_ac: string;
            child_id: string;
            child_revision: number;
            parent_ac: string;
            step_id: string;
          }[];
          full_chain_acs: string[];
          group_budget: { model_seconds: number; tokens: number; turns: number } | null;
          items: {
            budget: { model_seconds: number; tokens: number; turns: number };
            child_id: string;
            integration?: {
              configuration_sha256: string;
              repositories: {
                repair_scope: string;
                repository_id: number;
                repository_version: number;
                selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
              }[];
            } | null;
            merged_baseline_review: string;
            repair_scope: string;
            repository_version: number;
            revision: number;
            verification: {
              ac_id: string;
              step: {
                check: string;
                expected_result: string;
                id: string;
                selector: string;
                timeout_seconds: number;
              };
            }[];
          }[];
          parent_revision: number;
          semantic_review: string;
        };
        version: number;
      } | null;
      queue: { authorization_id: number; state: string; version: number } | null;
      repositories: {
        id: number;
        repository: {
          base_branch: string;
          environment?: string | null;
          github_repository_id: number;
          hooks?: {
            argv: string[];
            event: 'after_create' | 'after_run' | 'before_remove' | 'before_run';
            name: string;
            output_limit_bytes: number;
            replay?: 'idempotent' | 'never' | 'reconcile';
            roles: ('coding' | 'repair' | 'validation')[];
            script_identity: string;
            timeout_seconds: number;
          }[];
          model?: string | null;
          policy: {
            allowed_checks: string[];
            gate_recovery_policy: string;
            max_timeout_seconds: number;
            model_work_seconds: number;
            token_limit: number;
            turn_limit: number;
          };
          project: string;
          reason: string;
          remote: string;
          revoked: boolean;
        };
        version: number;
      }[];
      review: {
        coverage: {
          child_ac: string;
          child_id: string;
          child_revision: number;
          parent_ac: string;
          step_id: string;
        }[];
        full_chain_acs: string[];
        group_budget: { model_seconds: number; tokens: number; turns: number } | null;
        items: {
          budget: { model_seconds: number; tokens: number; turns: number };
          child_id: string;
          integration?: {
            configuration_sha256: string;
            repositories: {
              repair_scope: string;
              repository_id: number;
              repository_version: number;
              selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
            }[];
          } | null;
          merged_baseline_review: string;
          repair_scope: string;
          repository_version: number;
          revision: number;
          verification: {
            ac_id: string;
            step: {
              check: string;
              expected_result: string;
              id: string;
              selector: string;
              timeout_seconds: number;
            };
          }[];
        }[];
        parent_revision: number;
        semantic_review: string;
      } | null;
      scheduler_available: boolean;
      version: number;
    }
  | { error: string };
export interface SaveGroupReviewRequest {
  draft_revision: number;
  review: {
    coverage: {
      child_ac: string;
      child_id: string;
      child_revision: number;
      parent_ac: string;
      step_id: string;
    }[];
    full_chain_acs: string[];
    group_budget: { model_seconds: number; tokens: number; turns: number } | null;
    items: {
      budget: { model_seconds: number; tokens: number; turns: number };
      child_id: string;
      integration?: {
        configuration_sha256: string;
        repositories: {
          repair_scope: string;
          repository_id: number;
          repository_version: number;
          selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
        }[];
      } | null;
      merged_baseline_review: string;
      repair_scope: string;
      repository_version: number;
      revision: number;
      verification: {
        ac_id: string;
        step: {
          check: string;
          expected_result: string;
          id: string;
          selector: string;
          timeout_seconds: number;
        };
      }[];
    }[];
    parent_revision: number;
    semantic_review: string;
  };
  version: number;
}
export type AuthorizeGroupResponse =
  | {
      authorization_id: number;
      business_complete: boolean;
      scheduler_available: boolean;
      state: string;
    }
  | { error: string };
export interface AuthorizeGroupRequest { draft_revision: number; request_id: string; version: number }
export type EditGroupQueueResponse =
  { affected: string[]; version: number } | { error: string };
export interface EditGroupQueueRequest {
  change: {
    document?: {
      children: {
        acceptance_criteria: { description: string; id: string }[];
        depends_on: string[];
        goal: string;
        id: string;
        kind: 'code_change' | 'validation_only';
        order: number;
        parent_id: string;
        repository_id: number | null;
        validation_plan: string;
      }[];
      parent: {
        acceptance_criteria: { description: string; id: string }[];
        goal: string;
        id: string;
        scope: string;
      };
      schema: string;
    };
    edit_version?: number;
    kind: 'approve' | 'propose' | 'reorder';
    order?: string[];
    review?: {
      coverage: {
        child_ac: string;
        child_id: string;
        child_revision: number;
        parent_ac: string;
        step_id: string;
      }[];
      full_chain_acs: string[];
      group_budget: { model_seconds: number; tokens: number; turns: number } | null;
      items: {
        budget: { model_seconds: number; tokens: number; turns: number };
        child_id: string;
        integration?: {
          configuration_sha256: string;
          repositories: {
            repair_scope: string;
            repository_id: number;
            repository_version: number;
            selection: { kind: 'completed_dependencies' | 'fixed'; sha?: string };
          }[];
        } | null;
        merged_baseline_review: string;
        repair_scope: string;
        repository_version: number;
        revision: number;
        verification: {
          ac_id: string;
          step: {
            check: string;
            expected_result: string;
            id: string;
            selector: string;
            timeout_seconds: number;
          };
        }[];
      }[];
      parent_revision: number;
      semantic_review: string;
    };
  };
  request_id: string;
  version: number;
}
export interface AuthCsrfResponse { csrf_token: string; username: string | null }
export type AuthLoginResponse =
  { csrf_token: string; username: string | null } | { message: string } | undefined;
export interface AuthLoginRequest { password: string; username: string }
export type AuthSessionResponse = { csrf_token: string; username: string | null } | undefined;
export type AuthLogoutResponse = undefined;
