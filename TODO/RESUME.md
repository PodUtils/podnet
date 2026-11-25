**Resume work**

1. Use the checkout that the cloud provides. Claude Code calls
   `sh scripts/session-start.sh` with its session hook. Codex calls it
   from the checkout.
2. Read [the handoff](../docs/HANDOFF.md) if it exists, then
   [PROGRESS.md](PROGRESS.md) and [the owner's decisions](../docs/decisions.md).
   Check recent CI and issues. Use [INDEX.md](INDEX.md) for the entry
   and its verification.
3. Follow the [hosted setup](../docs/hosted-sessions.md) to load Rust and
   check the work record. Build the CLI when the task needs it.
