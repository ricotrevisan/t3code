# PROTOTYPE — tiny-model titles + ticket slugs in the T3 thread list

**Throwaway. Not production. Do not merge.**

Answers two questions:

1. Do titles from the 350M Desert Ant `title` model read well in T3's thread list
   next to the titles the agent providers produce today?
2. Can a Linear / GitHub ticket slug (`DEF-1916`, `#28`, `bubble_ex#22`) be folded
   into that title?

## Run it

```bash
uv run --with mlx-lm python prototype/title-model/titler.py   # ~40s, loads the model once
open prototype/title-model/index.html                          # double-click is fine too
```

`titler.py` writes a single self-contained `index.html` (data inlined, no server,
no build) plus `rows.json`. Bottom bar switches composition variant; the left
pane is the thread list, the right pane shows the input, today's title, the model
title, the slug arm, and the extracted reference.

Variants:

| | |
|---|---|
| V1 | model title, no slug |
| V2 | `#28: Adding pre-SSE authorization boundaries` (prefix) |
| V3 | `Adding pre-SSE authorization boundaries · #28` (suffix) |
| V4 | slug as a separate chip beside the title, not in the text |

Plus a toggle that shows the model's one-line description under each row.

## Findings

### 1. The slug cannot come from the prompt — build it by post-processing

The canonical instruction plus an explicit request for the slug
(`If a ticket reference such as DEF-1916 or #28 appears, start the title with it
in the form "DEF-1916: Title"`) over 5 ticket-bearing inputs:

- **0/5** used the requested prefix form.
- **4/5** mentioned the reference *somewhere*, inconsistently:
  - `Implement DEF-1916: participant updates` (front, but not the asked form)
  - `Fixing flaky test in ricotrevisan/bubble_ex#22` (tail)
  - `Implementation of #28 pre-SSE boundary` (middle)
  - `Issue 28 DSTAR compilation and site` — **normalized `#28` into "Issue 28"**, dropping the `#`

This is one more instance of the arm-B/C result from
`docs/ops/title-model-eval.md`: paraphrasing the single trained instruction gets
unreliable behaviour. **Compose the slug in code, leave the prompt alone.** The
prefix/suffix/chip variants all render fine once composed deterministically.

### 2. Extraction works on clean input, and the traps are real

13/13 correct on unambiguous inputs, including both decoys:

| input contains | extracted | note |
|---|---|---|
| `UTF-8`, `AES-256`, `SHA-256` | `None` | a bare `[A-Z]+-\d+` regex matches all three — project keys must be **allowlisted** (`DEF`, `OTP`, `ENG`), not pattern-matched |
| `&#8220;` | `None` | requires `(?<!&)`; a plain `#\d+` matches `8220` |
| `DEF-1916` | `DEF-1916` | Linear key |
| `ricotrevisan/bubble_ex#22` | `bubble_ex#22` | GitHub, owner dropped |
| `github.com/ricotrevisan/dstar/issues/28` | `dstar#28` | GitHub URL normalized |
| `epic #24 … #28: add a pre-SSE boundary` | `#28` | needed an imperative/lead-position rank, or the epic wins |

**4 of 17 inputs were genuinely ambiguous** — messages that reference 3–11
distinct refs, because they paste a whole ticket list (`#22 #25 #26 #27 #28
#29 #30 #32 #33 #34`). On those the pick is arguable: the S1 exporter message
resolved to `#32` where the subject is arguably `#27`. **Prose parsing cannot fix
this.** The durable answer is to take the slug from a structured source:
T3's own `linked_pull_request_json` / `branch_pull_request_json` / branch name,
or the tracker itself (Linear API / MCP). In this T3 instance
`linked_pull_request_json` is empty on every thread and every thread branch is
`main`, so prose is currently the only available signal — that is the real gap,
not the regex.

### 3. Title register, unchanged from the earlier eval

Still plainer and more process-flavoured than the provider titles: today's
`Implement S1 Frontend Exporter` vs. model `Implementing S1 frontend exporter for
BubbleEx`. It keeps `pre-SSE`, `BubbleEx`, `DStar`, `#28` exactly. Best single
case: a baseline that was a truncated copy of the user's message becomes
`Building a lean harness for OpenClaw`.

## What this does *not* answer

- Whether it looks right **inside the real T3 app** — this is a static mock of the
  sidebar row, not the app. The truncation behaviour is approximated with CSS
  `text-overflow: ellipsis`, and T3 truncates in a narrower column.
- Latency end-to-end through the server, or behaviour under concurrent titles.
- Where the real code goes. In production this is a `TextGenerationService`
  implementation beside `GrokTextGeneration.ts` / `ClaudeTextGeneration.ts`,
  calling `mlx_lm.server` and returning `{ title }` after `sanitizeThreadTitle`,
  with the slug composed from a structured field and a fail-open path to the
  current provider.

## Files

- `titler.py` — extraction + both prompt arms + HTML generation (the whole prototype)
- `index.html` — generated, self-contained, open this
- `rows.json` — generated data
- `t3_corpus.json` — 18 real T3 threads (title + first user message), pulled
  read-only from `~/.t3/userdata/state.sqlite` on `openclaw`
- `results.json` — the 43-input three-arm eval that preceded this
