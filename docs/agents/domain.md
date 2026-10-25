# Domain docs

## Before exploring

Read the vocabulary in `AGENTS.md` and `docs/internals/glossary.md`.
Consult `docs/internals/overview.md` and relevant internal documentation
for architectural constraints.

When present, also read root `CONTEXT.md` and relevant ADRs in `docs/adr/`.
If those optional documents are absent, proceed silently.

## Layout

Use a single context for the repository:

- `CONTEXT.md`: repository-wide domain context.
- `docs/adr/`: numbered architecture decision records.

Create these lazily through domain-modeling work when terms or decisions
are resolved. Do not scaffold empty documents during setup.

Preserve existing internal documentation. Link to established definitions
and decisions rather than copying them into a second source of truth.
Follow the documentation rules in `AGENTS.md`.

## Vocabulary and decisions

Use established domain terms in tickets, proposals, hypotheses, and tests.
If a needed concept is undefined, surface the gap during domain modeling.

Explicitly flag proposals that conflict with an existing documented
decision, identify the source, and explain why it should be reconsidered.
