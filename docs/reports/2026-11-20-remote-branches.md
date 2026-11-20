# Remote branches of PodUtils/podnet, 2026-11-20

Read on 2026-11-20 (UTC) with the GitHub REST API: `branches?per_page=100` (page 1 has one branch, page 2 has none),
`git/matching-refs/heads`, `pulls?state=open` (no pull request), and `git ls-remote --heads origin`. The four
answers agree.

| Branch | Last commit date (UTC) | ahead_by | behind_by | Open PR | Verdict | Reason |
| --- | --- | --- | --- | --- | --- | --- |
| `main` | 2026-11-20 00:19 | 0 | 0 | none | keep | The default branch. It is never deleted. |

No branch other than `main` exists. No branch has the verdict "delete".

Notes:

* The pull requests of Dependabot (#557, #556, #415, #383, #382, #381, #318, #315, #314, #313, #312, #30 to #27) are all
  closed. Their branches are already deleted.
* The work of parallel agents uses local worktree branches. They are not pushed (`CLAUDE.md`, "Git").
* `scripts/agent/delete-stale-branches.ps1` deletes the branches that this table marks "delete". With no branch given,
  it does nothing. Run this check again before a use of the script.
