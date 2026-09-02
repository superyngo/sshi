# Reference

Current behavior only. Anything historical — a superseded design, a shipped plan, a resolved
investigation — lives in `../spec/`, `../plan/`, `../debug/`, or `../audit/`, not here.

- **[glossary.md](glossary.md)** — canonical vocabulary; read first.
- **[cli.md](cli.md)** — every subcommand, global/target flags, and exit behavior.
- **[config-schema.md](config-schema.md)** — `config.toml` schema and `~/.ssh/config` parsing rules.
- **[ssh-transport.md](ssh-transport.md)** — russh-based session pool, auth chain, SFTP transfer.
- **[sync-algorithm.md](sync-algorithm.md)** — the 3-stage sync protocol (collect / decide / distribute).
- **[state-schema.md](state-schema.md)** — SQLite schema, migrations, and retention policy.
- **[tui.md](tui.md)** — TUI tab structure, keybindings, and contributor rules.

Machine-checked: `cli.md`'s subcommand/flag claims by `cargo test --lib cli::tests`
(`src/cli.rs`).

See also [`../adr/`](../adr/README.md) for decision records.
