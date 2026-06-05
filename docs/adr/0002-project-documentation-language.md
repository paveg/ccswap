# ADR-0002: Project documentation language

## Status

Accepted

## Context

The project has design docs, ADRs, agent instructions, npm packaging metadata,
and code comments that may be read by contributors and release consumers across
different environments. Mixing languages in long-lived repository files makes
search, review, and future packaging work harder.

The maintainer may still prefer Japanese for interactive development
conversation.

## Decision

Write project-facing documentation in English. This includes:

- `CLAUDE.md`
- `AGENTS.md`
- `docs/`
- ADRs
- package/release docs
- durable code comments

Interactive conversation with the maintainer can be Japanese.

## Consequences

- Repository history and project docs stay consistent and easier to search.
- ADRs can be referenced directly from release, npm, or contributor docs without
  translation.
- Short-lived discussion can remain in Japanese, but accepted decisions should be
  converted into English ADRs before they become project policy.
