// harness-contract-sha256: 0215416798bafcba0ff1d89defcb583c833fc04879c56446bc63347ff7b5ea76
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
