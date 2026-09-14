# CodexSymphony Architecture Boundaries

Status: **Normative responsibility boundaries** · 2026-09-14

This document defines responsibility, not implementation scope. Current behavior and acceptance live in the [main specification](../Personal_AI_Software_Factory_综合方案.md). Deferred framework designs have no current normative force.

## 1. Three truth domains

| Domain | Owns |
|---|---|
| Requirement | Reviewed input, business lifecycle and acceptance |
| AgentRun | Execution attempts, processes, completion declarations and consumption |
| GitHub / CI observations | Observed repository, PR, check and merge facts |

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
