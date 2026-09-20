// harness-contract-sha256: 03fb4558975f8691e73d48629a90caeffad41a4f35b84d49049e499c2b3d390a
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
      github_repository_id: number;
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
        github_repository_id: number;
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
    github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
export interface PauseExecutionRequest {
  pause: true;
}
export type PauseRequirementResponse = { paused: true } | { error: string };
export interface PauseRequirementRequest {
  pause: true;
}
export type GetOperationsResponse =
  | {
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
        cached: number | null;
        human_seconds: number | null;
        input: number | null;
        interventions: number;
        model_calls: number;
        output: number | null;
        phases: { complete: boolean; phase: string; seconds: number }[];
        reasons: string[];
        repair_count: number;
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
export interface GetInboxResponse {
  requirement_ids: number[];
}
export type AnswerOperatorQuestionResponse = { saved: boolean } | { error: string };
export interface AnswerOperatorQuestionRequest {
  answers: { id: string; text: string }[];
  version: number;
}
export interface MultiGetRepositoryResponse {
  deployment_network: string[];
  network_status: string;
  repositories: {
    delivery_ready?: boolean;
    id?: number;
    repository: {
      base_branch: string;
      github_repository_id: number;
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
        github_repository_id: number;
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
    github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
          github_repository_id: number;
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
