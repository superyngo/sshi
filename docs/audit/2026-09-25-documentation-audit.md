# Documentation audit
Status: Resolved (2026-09-25)

Two-pass sweep (structure, then accuracy against `src/`) of every living document after the
2026-09-02 layout migration (ADR 0003). Per `wens-dev-principles docs 19`.

## Pass 1 — structure

| # | Finding | Fix |
|---|---|---|
| S1 | No living backlog; follow-ups scattered across `plan/2026-07-18-audit-fixes.md`, ADR 0001/0002, `audit/2026-05-21-tui-audit.md` | [`plan/BACKLOG.md`](../plan/BACKLOG.md) created |
| S2 | `CONTEXT.md` missing backlog row and reading-order step; audit/tmp rows incomplete | Updated |
| S3 | ADR 0003 index status `Accepted` not in value set | `Implemented (2026-09-02)` |
| S4 | Dead paths in ADR 0002, `audit/2026-05-21-readme-analysis.md`, v1.7.0 changelog entry | Mechanical path repair |
| S5 | 13 `src/` doc comments cite dead `docs/tui_reconstruct_plan.md` | Repointed to `docs/spec/2026-05-06-tui-reconstruct.md` |
| S6 | `reference/README.md` claimed `cli.md` is machine-checked; no test reads docs | Claim removed |
| S7 | Line-number citation in `state-schema.md` | Symbol citation |
| S8 | Glossary `SessionPool` entry broke the entry format | Fixed |
| S9 | Changelog prose sub-heading `### Unreleased Update — …` | `### 2026-09-02` |
| S10 | Root `CHANGELOG.md` held 13 v0.x releases | Archived verbatim to `reference/changelog/v0.x.md` |
| S11 | Contributor rules duplicated in `tui.md` and `AGENTS.md`; Copilot file duplicated commands | Rules only in `AGENTS.md`; Copilot file is a pointer |
| S12 | Stray tracked root files `CHANGELOG`, `temp`, `.claude/scheduled_tasks.lock` | Removed; `.claude/` ignored |

## Pass 2 — accuracy

All corrected in `docs/reference/`, `README.md`, `AGENTS.md`:

- **Build features.** `AGENTS.md`, `README.md`, `tui.md` said source builds are headless; `Cargo.toml`
  defaults to `tui`. Headless is `--no-default-features`. `AGENTS.md` also named a nonexistent
  `feat/tui` merge branch.
- **`tui.md`.** Invented Config host fields; wrong CheckEnabled probe catalog; wrong Operate
  operation order and layout; tab-bar filter badge that does not exist; MemberPicker `s` and
  DirectVecEditor `e` keys that do not exist; wrong View selector label and checkout columns;
  wrong `state_dir` derivation.
- **`cli.md`.** Target-validation exit code (1, not 2); `-v` is top-level only; flag matrix
  missing flags; ignored/implicit flag behaviour not stated.
- **`config-schema.md`.** Unnamed `[[check]]` entries are unselectable.
- **`ssh-transport.md`.** `init` does not use `SshPool`; `Context` not `CommandContext`;
  `SessionPool` trait missing; permit order in `distribute_pooled`.
- **`sync-algorithm.md` / `state-schema.md`.** Missing fourth sync phase; invented
  `SyncSummary` fields; `sync_state` placeholder values and write-only use; `operation_log`
  status values.
- **`README.md`.** Removed `run --yes`; config example on legacy schema; `list` undocumented;
  sections duplicating `CHANGELOG.md` and `tui.md` replaced by links.
- **Glossary.** `SyncEntry` does not hold `ConflictStrategy`; nine missing terms added.

Code defects surfaced by this audit are tracked in the backlog as B13–B19, not fixed here.

## Not changed

- Frozen records keep historical wording (e.g. the second `Status:` text on line 4 of
  `plan/2026-05-21-unified-schema-refactor.md`); only path strings were repaired.
- `docs/tmp/` stays gitignored per ADR 0003 (`wens-dev-principles docs 12` is CONSIDER).
