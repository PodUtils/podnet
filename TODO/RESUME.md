**Resume work**

1. Use the checkout that the cloud provides. Claude Code starts it
   through its session hook. In Codex, run `sh scripts/session-start.sh`
   from the checkout and load `. ./scripts/cloud-env.sh` in each shell.
   In Antigravity / Gemini Cloud, `.agents/environment.json` runs
   `sh scripts/session-start.sh --no-watchers` and
   `sh scripts/cloud-setup.sh --dev-tools` on VM startup; run
   `sh scripts/session-start.sh --no-watchers` if needed, start both
   watchers as daemon tasks (`--run-watcher watch` and
   `--run-watcher runs`), and load `. ./scripts/cloud-env.sh` in each shell.
2. Read [the handoff](../docs/HANDOFF.md) if it exists, then
   [PROGRESS.md](PROGRESS.md) and [the owner's decisions](../docs/decisions.md).
   Check recent CI and issues. Follow the [Actions procedure](../docs/maintenance.md#run-github-actions)
   if the work needs a manual run. Use [INDEX.md](INDEX.md) for the entry
   and its verification.
3. Follow the [hosted setup](../docs/hosted-sessions.md) to load Rust and
   check the work record. Build the CLI when the task needs it.
