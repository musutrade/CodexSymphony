# CodexSymphony Architecture Boundaries

Status: **Normative responsibility boundaries** · updated 2026-09-24

This document defines responsibility, not implementation scope. Current behavior and acceptance live in the [main specification](../Personal_AI_Software_Factory_综合方案.md). Deferred framework designs have no current normative force.

## 1. Three truth domains

| Domain | Owns |
|---|---|
| Requirement | Reviewed input, business lifecycle and acceptance |
| AgentRun | Execution attempts, processes, completion declarations and consumption |
| Delivery observations (currently GitHub / CI) | Observed repository, PR, check and merge facts; local delivery facts only when that mode is implemented and enabled |

Keep these independently queryable and reconcilable. A single coordinator does not imply a single status for all domains.

## 2. Gate result is validation evidence

Harness-Gate may authoritatively evaluate its configured policy for an exact source/config/evidence identity.
CodexSymphony consumes that result as evidence, not a fourth lifecycle or a substitute for business acceptance.
Evidence preserves source identity, trusted configuration identity, result and artifact references; current storage and validation behavior are defined in the main specification.

## 3. Forbidden inference

The following facts alone do not justify the corresponding state change:

- Gate PASS → Requirement Done.
- Gate FAIL → a terminal AgentRun rewritten as Failed.
- AgentRun Succeeded → PR merged.
- PR merged → business acceptance completed.
- Local commit exists → remote branch or PR exists.
- A previous SHA passed → the current SHA passed.

A valid agent completion and preserved candidate can end execution successfully while independent validation fails.
The orchestrator records which policy and observed evidence justify each next action.
Queue release is an orchestration decision; observing a merge for queue release does not fabricate business Done.

## 4. Division of responsibility

CodexSymphony owns scheduling, execution supervision, authorization, recovery budgets, external action reconciliation and business decisions.
Harness-Gate owns quality semantics, including CRAP, coverage, baseline/ratchet/debt and collector trust. CodexSymphony does not reimplement them.
Project-specific API, integration, E2E and other tests remain owned by the project. Generic validation hooks do not replace project test frameworks.
Platform credentials and remote actions remain outside the agent's delegated tools; actual isolation limits are explicit in the main specification.

## 5. No framework implied

These boundaries do not require separate services, a table per conceptual record, event sourcing, dual leases, diagnostic agents or a resource platform.
Implement the minimum current behavior while preserving independent facts and exact provenance.
Future model diagnosis remains advisory: a hypothesis is not a confirmed cause, authorization or permission to change a budget.

## 6. Review questions

An implementation should independently answer:

- What was required and authorized?
- What did the agent execute and preserve?
- What did GitHub/CI actually report?
- Which exact source/configuration did validation cover?
- Which orchestration rule authorized the next action?

Behavioral rules have one authoritative definition in the main specification; this file does not repeat their state machines or transaction algorithms.

## 7. Trusted development environment (2026-09-16)

Development follows Symphony's trusted-environment model. Ordinary Agent commands,
Git, builds, databases, browsers and real Runtime tests use that environment directly.
No per-command sandbox, mandatory static network allowlist, nested validation
namespace, reviewed binary manifest or per-test host receipt is a product requirement.
The deployment may provide a single account/environment boundary; it is not a
project-specific execution protocol. This phase does not run adversarial code.

GitHub credentials stay in the authorized remote-operation service. Gate signing
keys stay in the independent verifier, which fetches an exact commit and evaluates
approved policy. Neither service's credentials enter the development environment.
Development test output is not a signed Gate decision. Budgets, candidate identity,
recovery, real CI checks and external-action authorization remain in force.

## 8. Extension boundaries and implementation scope

The core owns authorization, scheduling, budgets, process quiescence, preservation, independent acceptance and reconciliation. Project hooks supply preparation and auxiliary cleanup; execution adapters supply Agent interactions; delivery adapters supply observed delivery facts; optional decision adapters supply advice. Platform PostgreSQL is not a mandatory dependency of managed projects.

The [extension requirements](extension-requirements.md) own staged scope and acceptance; the [extension protocol](extension-protocol.md) owns interface and lifecycle semantics. The original types and four lifecycle hooks were delivered by #103/#104; environment/validation/delivery integration and task model selection remain staged work. Existing frozen tasks retain their policy; merging documentation does not start the paused queue.

The extension work does not require a plugin marketplace, a service per adapter, or moving authoritative preservation into best-effort scripts. The development controller's WORKFLOW.lifecycle.md hooks are separate from the Rust product interfaces described here.

## 9. Controlled delivery contract (GH-118)

The [unified extension protocol](extension-protocol.md#p9-受控环境验证与交付) is the
single source for operation names, versions and environment/validation/delivery
result semantics. The core freezes authorization and candidate identity, supervises
calls, checks provenance/completeness, preserves evidence and reconciles unknown
side effects. Reviewed extensions implement environment probes, complete validation,
credential provision and delivery. Core configuration holds capability/scope/config
references, not vendor authentication fields or mandatory PR/CI facts.

Rust, a project database, GitHub App, Harness-Gate and build/dependency caches are
not universal project requirements. This repository retains its own required Gate.
Candidate execution receives no delivery secrets; credentialed extensions execute
only reviewed implementation, never Agent-modifiable scripts. Agent declarations
cannot approve acceptance. Four lifecycle hooks retain their existing semantics.

Submitted and Done describe applicable delivery and business acceptance independently
of hosting provider and execution stages. Local delivery has no invented PR/CI/merge
facts. P10 maps existing call sites and planned integration: GH-118 provides contracts
and compatibility checks, not the later adapters or production deployment.
