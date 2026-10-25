# Issue tracker: GitHub

Maintainer issues and specs live in GitHub Issues for `ricotrevisan/t3code`.

Use `gh` with explicit `--repo ricotrevisan/t3code` for issue operations.
For API calls, explicitly target `repos/ricotrevisan/t3code/...`.
The upstream `pingdotgg/t3code` remote is not this fork's tracker.

## Operations

- Publish a ticket: `gh issue create --repo ricotrevisan/t3code --title "..." --body-file <file>`
- Fetch a ticket: `gh issue view <number> --repo ricotrevisan/t3code --comments`
- List tickets: `gh issue list --repo ricotrevisan/t3code --state open --json number,title,body,labels,assignees`
- Comment: `gh issue comment <number> --repo ricotrevisan/t3code --body-file <file>`
- Apply or remove labels: `gh issue edit <number> --repo ricotrevisan/t3code --add-label "..." --remove-label "..."`
- Close: `gh issue close <number> --repo ricotrevisan/t3code`

Write multiline bodies to a temporary file outside the worktree and use
`--body-file`. Read the ticket's current body, labels, and comments before
updating it.

Use `docs/agents/triage-labels.md` for triage roles.
Follow `AGENTS.md` for contribution, PR, and work-artifact rules.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Wayfinding

A map is one issue labelled `wayfinder:map`, with Notes, Decisions-so-far,
and Fog sections. Child tickets use `wayfinder:research`,
`wayfinder:prototype`, `wayfinder:grilling`, or `wayfinder:task`.

Link children using GitHub sub-issues. If unavailable, use a task list in
the map and a `Part of #<map>` reference in each child.

Represent blockers with native GitHub issue dependencies. If unavailable,
use a `Blocked by: #<number>` line in the child. A child is unblocked when
all its blockers are closed.

The frontier is the first open, unassigned, unblocked child in map order.
Claim it by assigning the driving developer. On resolution, record the
answer on the child, close it, and link the result from Decisions-so-far.
