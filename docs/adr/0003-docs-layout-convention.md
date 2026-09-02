# ADR 0003 — Adopt wens-dev-principles docs layout convention

**Status:** Accepted
**Date:** 2026-09-02

## Context

Prior documentation across the repository was fragmented and difficult to navigate:
- Documentation files were scattered across `docs/superpowers/{specs,plans,audits}/`, `docs/plans/`, `docs/ai-reports/`, and an untracked file stranded in gitignored `docs/tmp/`.
- Documents lacked lifecycle tracking (`Status:` headers), making it ambiguous whether a document was an active design, implemented plan, or historical proposal.
- Two agent-instruction files (`AGENTS.md` and `.github/copilot-instructions.md`) had drifted out of sync with each other and the codebase, containing stale and contradictory technical descriptions.

## Decision

Adopt the fixed `docs/{reference,adr,spec,plan,debug,audit,tmp}/` directory layout and documentation lifecycle management per `wens-dev-principles docs` principles 1 through 15:

1. **Fixed directory structure**:
   - `docs/reference/`: Single source of truth for current behavior and architecture.
   - `docs/adr/`: Numbered architecture decision records (expensive to decide/reverse).
   - `docs/spec/`: Requirements and design specifications (frozen on landing).
   - `docs/plan/`: Implementation step plans (frozen on landing).
   - `docs/debug/`: Systematic debugging investigation records.
   - `docs/audit/`: Point-in-time codebase sweeps and reviews.
   - `docs/tmp/`: Scratch/wip documents (gitignored).
2. **Root index**: Maintain root `CONTEXT.md` as the primary onboarding entry point.
3. **Lifecycle status**: Require a standard `Status:` line as line 2 of every document in `spec/`, `plan/`, `debug/`, and `audit/`.
4. **Instruction files**: Keep `AGENTS.md` and `.github/copilot-instructions.md` focused strictly on conduct and contributor workflows, delegating technical reference facts to `docs/reference/`.

## Consequences

- **Paired naming**: Future specifications and implementation plans must pair by identical kebab-case filename (e.g. `docs/spec/YYYY-MM-DD-title.md` and `docs/plan/YYYY-MM-DD-title.md`).
- **Immutability of ADRs**: ADRs are numbered sequentially (`0001-...md`, `0002-...md`, etc.) and are never edited after acceptance, except for repairing broken relative links when referenced documents are relocated.
- **Single source of truth**: `docs/reference/` is the sole authoritative documentation for current behavior, configuration schemas, transport protocols, and CLI surfaces.
