import { GetGroupReviewResponse, SaveGroupReviewRequest } from './health-response';
export type GroupView = Extract<GetGroupReviewResponse, { draft_id: string }>;
export type Review = SaveGroupReviewRequest['review'];
export const zeroBudget = () => {
  return { tokens: 0, turns: 0, model_seconds: 0 };
};
export function totalBudget(items: Review['items']) {
  return items.reduce((total, item) => {
    return {
      tokens: total.tokens + item.budget.tokens,
      turns: total.turns + item.budget.turns,
      model_seconds: total.model_seconds + item.budget.model_seconds,
    };
  }, zeroBudget());
}
export function repairLimits(view: GroupView) {
  return view.document.children.map((child) => {
    const authorization = view.authorizations.find((entry) =>
      entry.snapshot.document.children.some((original) => original.id === child.id),
    );
    const repositories = authorization?.snapshot.repositories ?? view.repositories;
    const repository = repositories.find((entry) => entry.id === child.repository_id);
    return {
      childId: child.id,
      limit: repository?.repository.policy.gate_recovery_policy === 'bounded_v1' ? 3 : 1,
    };
  });
}
export function initialReview(view: GroupView): Review {
  const items = view.document.children.map((child) => {
    const repo = view.repositories.find((r) => r.id === child.repository_id);
    return {
      child_id: child.id,
      revision: view.draft_revision,
      repository_version: repo?.version ?? 0,
      budget: {
        tokens: repo?.repository.policy.token_limit ?? 0,
        turns: repo?.repository.policy.turn_limit ?? 0,
        model_seconds: repo?.repository.policy.model_work_seconds ?? 0,
      },
      repair_scope: '',
      merged_baseline_review: '',
      verification: child.acceptance_criteria.map((ac, i) => {
        return {
          ac_id: ac.id,
          step: {
            id: `verify-${i + 1}`,
            check: repo?.repository.policy.allowed_checks[0] ?? 'cargo_test',
            selector: '',
            expected_result: 'exit 0',
            timeout_seconds: Math.min(repo?.repository.policy.max_timeout_seconds ?? 60, 60),
          },
        };
      }),
    };
  });
  return {
    parent_revision: view.draft_revision,
    full_chain_acs: view.document.parent.acceptance_criteria.map((a) => a.id),
    coverage: [],
    items,
    group_budget: null,
    semantic_review: '',
  };
}
export function editableReview(review: Review) {
  return {
    ...review,
    group_budget: review.group_budget ?? totalBudget(review.items),
    automatic_budget: review.group_budget === null,
  };
}
