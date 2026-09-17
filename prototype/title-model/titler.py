#!/usr/bin/env python3
"""PROTOTYPE - throwaway. Answers: do tiny-model titles read well in T3's
thread list, and can a Linear/GitHub ticket slug be folded into them?

Run:  uv run --with mlx-lm python prototype/title-model/titler.py
Then: open prototype/title-model/index.html
"""

import html
import json
import re
from pathlib import Path

from mlx_lm import generate, load

HERE = Path(__file__).parent

CANON = (
    "Write a factual title (3-8 words) and a 1-2 sentence description for this passage."
    "Be specific enough to identify this passage. No emoji, no hashtags, no hype."
    "Write in the same language as the passage.\n\nPASSAGE:\n{clip}"
)

# Arm 2: the same instruction plus a request to carry the ticket reference.
CANON_SLUG = (
    "Write a factual title (3-8 words) and a 1-2 sentence description for this passage."
    "Be specific enough to identify this passage. No emoji, no hashtags, no hype."
    "Write in the same language as the passage."
    ' If a ticket reference such as DEF-1916 or #28 appears, start the title with it'
    ' in the form "DEF-1916: Title".\n\nPASSAGE:\n{clip}'
)

# --- ticket reference extraction -------------------------------------------
# Evidence-driven: a bare [A-Z]+-\d+ regex matches UTF-8, AES-256 and COVID-19 in
# real session text, so project keys are allowlisted, not pattern-matched.
LINEAR_KEYS = {"DEF", "OTP", "ENG"}

GITHUB_URL = re.compile(r"github\.com/([\w.-]+)/([\w.-]+)/(?:issues|pull)/(\d+)")
OWNER_REPO_HASH = re.compile(r"\b([\w.-]+)/([\w.-]+)#(\d+)\b")
KEYED = re.compile(r"\b([A-Z][A-Z0-9]{1,9})-(\d{1,5})\b")
# (?<!&) rejects the "&#8220;" HTML-entity shape seen in real transcripts.
BARE_HASH = re.compile(r"(?<!&)(?<![\w#])#(\d{1,5})\b")
ISSUE_CONTEXT = re.compile(r"issue|epic|ticket|child of|blocked by|slic|#\d")
# A reference a user actually means is usually the thing being acted on, so a
# leading position or an imperative verb outranks a passing mention.
IMPERATIVE = re.compile(r"implement|pick up|fix|address|start|ship|close|revert|bump|do\b|review")


def _hash_score(text, m):
    """Rank a bare #N mention; None means do not use it."""
    before = text[max(0, m.start() - 40) : m.start()].lower()
    if m.start() < 80 or IMPERATIVE.search(before):
        return 3
    if ISSUE_CONTEXT.search(before):
        return 1
    return None


ALL_REFS = re.compile(
    r"github\.com/[\w.-]+/[\w.-]+/(?:issues|pull)/\d+|\b[\w.-]+/[\w.-]+#\d+\b"
    r"|\b[A-Z][A-Z0-9]{1,9}-\d{1,5}\b|(?<!&)(?<![\w#])#\d{1,5}\b"
)


def extract_ref(text):
    m = GITHUB_URL.search(text)
    if m:
        return f"{m.group(2)}#{m.group(3)}", "github-url"
    m = OWNER_REPO_HASH.search(text)
    if m:
        return f"{m.group(2)}#{m.group(3)}", "owner/repo#N"
    for m in KEYED.finditer(text):
        if m.group(1) in LINEAR_KEYS:
            return f"{m.group(1)}-{m.group(2)}", "linear-key"
    best = None
    for m in BARE_HASH.finditer(text):
        score = _hash_score(text, m)
        if score is not None and (best is None or score > best[0]):
            best = (score, f"#{m.group(1)}", m.start())
    if best:
        why = "bare #N at lead/imperative" if best[0] == 3 else "bare #N in issue context"
        return best[1], why
    return None, None


# --- the shim ---------------------------------------------------------------


def clean_input(text):
    """Replicate what the harnesses' own titlers already strip."""
    text = re.sub(r"\[Triggering message id:[^\]]*\]", " ", text)
    text = re.sub(r"\[Thread context[^\]]*\]", " ", text)
    text = re.sub(r"\[(?:rico|Rico Trevisan[^\]]*)\]", " ", text)
    text = re.sub(r"\[[A-Za-z][^\]]{0,60}\]", " ", text)
    return re.sub(r"\s+", " ", text).strip()[:1000]


def parse_card(raw):
    t = re.search(r"TITLE:\s*(.+)", raw)
    d = re.search(r"DESC:\s*(.+)", raw, re.S)
    return (t.group(1).strip() if t else None, d.group(1).strip() if d else None)


def compose(title, ref, style):
    if not ref or style == "plain":
        return title
    if style == "prefix":
        return f"{ref}: {title}"
    if style == "suffix":
        return f"{title} · {ref}"
    return title


# --- inputs -----------------------------------------------------------------

DECOYS = [
    (
        "The export crashes on a UTF-8 file, and the key is AES-256, not SHA-256."
        " Can you look?",
        None,
    ),
    ("the JSON has a &#8220; smart quote in it and the parser chokes on it", None),
]

TICKET_CASES = [
    (
        "Implement ◉ #28: add a pre-SSE authorization boundary for page events and streams."
        " It is the next child of epic #24 after #25-#29.",
        "#28",
    ),
    ("let's pick up #30. Is it an implementation or grilling or design?", "#30"),
    ("Implement DEF-1916: every participant should get an updated invite.", "DEF-1916"),
    ("Fix the flaky test called out in ricotrevisan/bubble_ex#22", "bubble_ex#22"),
    ("https://github.com/ricotrevisan/dstar/issues/28 - can you check if it compiles and the site runs?", "dstar#28"),
] + DECOYS


def main():
    real = json.load(open(HERE / "t3_corpus.json"))[:10]
    rows = [{"msg": r["first_user"], "baseline": r["title"], "provider": r["provider"]} for r in real]
    rows += [{"msg": m, "baseline": None, "provider": None} for m, _ in TICKET_CASES]
    expected = [None] * len(real) + [e for _, e in TICKET_CASES]
    seen, deduped = set(), []
    for row, exp in zip(rows, expected):
        key = clean_input(row["msg"])
        if key in seen:
            continue
        seen.add(key)
        deduped.append((row, exp))
    rows = [r for r, _ in deduped]
    expected = [e for _, e in deduped]

    print("loading model ...")
    model, tok = load("desert-ant-labs/title")

    def gen(prompt, max_tokens=96):
        p = tok.apply_chat_template([{"role": "user", "content": prompt}], add_generation_prompt=True)
        return generate(model, tok, prompt=p, max_tokens=max_tokens, verbose=False).strip()

    out = []
    slug_ok = slug_present = slug_total = 0
    for i, (row, exp) in enumerate(zip(rows, expected)):
        passage = clean_input(row["msg"])
        ref, how = extract_ref(passage)

        plain = parse_card(gen(CANON.format(clip=passage)))
        slugged = parse_card(gen(CANON_SLUG.format(clip=passage)))

        slug = slugged[0] or ""
        if exp:
            slug_total += 1
            # Two separate questions: did the model put the ref at the FRONT in
            # the requested form, and did it carry the ref at all?
            if slug.lower().startswith(exp.lower()):
                slug_ok += 1
            if exp.lower() in slug.lower():
                slug_present += 1
        extraction_right = (ref == exp) if exp else (ref is None)
        n_refs = len(set(ALL_REFS.findall(passage)))
        out.append(
            {
                **row,
                "msg": passage,
                "ref": ref,
                "ref_how": how,
                "n_refs": n_refs,
                "ambiguous": n_refs > 2,
                "expected_ref": exp,
                "extraction_right": extraction_right,
                "model_title": plain[0],
                "model_desc": plain[1],
                "slug_title": slugged[0],
                "slug_desc": slugged[1],
                "variants": {
                    "plain": compose(plain[0], ref, "plain"),
                    "prefix": compose(plain[0], ref, "prefix"),
                    "suffix": compose(plain[0], ref, "suffix"),
                },
            }
        )
        print(f"[{i + 1}/{len(rows)}] ref={ref!r:14} model={plain[0]!r}")

    print(f"\narm 'slug in prompt': ref in requested prefix form {slug_ok}/{slug_total},"
          f" ref present anywhere {slug_present}/{slug_total}")
    amb = [r for r in out if r["ambiguous"]]
    scored = [r for r in out if not r["ambiguous"]]
    bad = [r for r in scored if not r["extraction_right"]]
    print(f"extraction: {len(scored) - len(bad)}/{len(scored)} matched expectation (unambiguous inputs)")
    for r in bad:
        print(f"   expected {r['expected_ref']!r} got {r['ref']!r}  <- {r['msg'][:70]}")
    for r in amb:
        print(f"   ambiguous input, {r['n_refs']} distinct refs -> picked {r['ref']!r}: {r['msg'][:60]}...")

    (HERE / "index.html").write_text(render(out))
    (HERE / "rows.json").write_text(json.dumps(out, indent=1))
    print(f"\nwrote {HERE / 'index.html'}")


def render(rows):
    data = json.dumps(rows)
    variants = [
        ("plain", "V1 - model title, no slug"),
        ("prefix", "V2 - slug prefix"),
        ("suffix", "V3 - slug suffix"),
        ("chip", "V4 - slug as separate chip"),
    ]
    buttons = "".join(
        f'<button data-v="{k}" onclick="setVariant(\'{k}\')">{html.escape(label)}</button>'
        for k, label in variants
    )
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><title>PROTOTYPE - title model in T3 thread list</title>
<style>
 :root {{ color-scheme: dark; }}
 body {{ margin:0; font: 13px/1.45 -apple-system, system-ui, sans-serif; background:#111114; color:#e7e7ea; display:flex; height:100vh; overflow:hidden; }}
 .pane {{ overflow:auto; padding:16px 20px; }}
 .left {{ width:320px; border-right:1px solid #26262c; background:#0d0d10; flex:none; }}
 .right {{ flex:1; }}
 h2 {{ font-size:11px; text-transform:uppercase; letter-spacing:.08em; color:#8b8b95; margin:0 0 10px; }}
 .row {{ padding:7px 8px; border-radius:7px; cursor:pointer; }}
 .row:hover {{ background:#1a1a20; }}
 .row.sel {{ background:#232331; }}
 .title {{ white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }}
 .sub {{ color:#8b8b95; font-size:11px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }}
 .chip {{ display:inline-block; background:#2b2b3a; color:#b9b9d6; border-radius:5px; padding:0 5px; margin-right:6px; font-size:11px; font-variant-numeric:tabular-nums; }}
 .desc {{ color:#a0a0ab; font-size:12px; }}
 .box {{ background:#16161b; border:1px solid #26262c; border-radius:9px; padding:12px 14px; margin-bottom:14px; }}
 .lbl {{ font-size:10px; text-transform:uppercase; letter-spacing:.07em; color:#7c7c88; margin-bottom:5px; }}
 .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size:11.5px; color:#c9c9d4; }}
 .bad {{ color:#ff8f8f; }} .good {{ color:#8fdca0; }}
 #bar {{ position:fixed; bottom:14px; left:50%; transform:translateX(-50%); background:#1c1c24; border:1px solid #33333d;
        border-radius:999px; padding:7px 9px; display:flex; gap:6px; box-shadow:0 8px 30px #0009; }}
 #bar button {{ background:#26262f; color:#cfcfda; border:1px solid #3a3a46; border-radius:999px; padding:6px 12px; cursor:pointer; font-size:12px; }}
 #bar button.on {{ background:#4c6ef5; border-color:#4c6ef5; color:#fff; }}
 .warn {{ color:#ffcc66; }}
</style></head><body>
<div class="pane left">
  <h2>T3 thread list - prototype</h2>
  <div id="list"></div>
</div>
<div class="pane right">
  <h2>Selected thread</h2>
  <div id="detail"></div>
</div>
<div id="bar">{buttons}
  <button onclick="toggleDesc()" id="db">description: off</button>
</div>
<script>
const ROWS = {data};
let variant = "plain", showDesc = false, sel = 0;

function titleOf(r) {{
  if (variant === "plain") return r.model_title || "(none)";
  return r.variants[variant] || r.model_title || "(none)";
}}
function setVariant(v) {{ variant = v; render(); }}
function toggleDesc() {{ showDesc = !showDesc; document.getElementById("db").textContent = "description: " + (showDesc ? "on" : "off"); render(); }}

function render() {{
  document.querySelectorAll("#bar button[data-v]").forEach(b => b.classList.toggle("on", b.dataset.v === variant));
  document.getElementById("list").innerHTML = ROWS.map((r, i) => {{
    const chip = variant === "chip" && r.ref ? `<span class="chip">${{r.ref}}</span>` : "";
    const desc = showDesc && r.model_desc ? `<div class="sub">${{r.model_desc.slice(0, 70)}}</div>` : "";
    const branch = r.provider ? `${{r.provider}} · main` : "no provider";
    return `<div class="row ${{i === sel ? "sel" : ""}}" onclick="sel=${{i}};render()">
      <div class="title">${{chip}}${{titleOf(r)}}</div>
      <div class="sub">${{branch}}</div>${{desc}}</div>`;
  }}).join("");
  const r = ROWS[sel];
  const refLine = r.ref
    ? `<span class="good">${{r.ref}}</span> <span class="sub">via ${{r.ref_how}}</span>`
    : `<span class="sub">no reference found</span>`;
  const exp = r.expected_ref === undefined || r.expected_ref === null ? "(non-ticket input)" : r.expected_ref;
  document.getElementById("detail").innerHTML = `
   <div class="box"><div class="lbl">input (first user message)</div><div class="mono">${{r.msg}}</div></div>
   <div class="box"><div class="lbl">today's title (from the agent provider)</div>${{r.baseline ? r.baseline : "<span class=\\"sub\\">- new thread, no title yet -</span>"}}</div>
   <div class="box"><div class="lbl">model title (canonical prompt)</div>${{r.model_title}}
      <div class="desc">${{r.model_desc || ""}}</div></div>
   <div class="box"><div class="lbl">model title (slug instruction added to prompt)</div>
      ${{r.slug_title}}<div class="desc">${{r.slug_desc || ""}}</div></div>
   <div class="box"><div class="lbl">extracted ticket reference</div>${{refLine}}
      <div class="sub">expected: ${{exp}}</div></div>
   <div class="box"><div class="lbl">composed for the list</div>
      <div>plain  : ${{r.variants.plain || "(none)"}}</div>
      <div>prefix : ${{r.variants.prefix || "(none)"}}</div>
      <div>suffix : ${{r.variants.suffix || "(none)"}}</div></div>`;
}}
render();
</script></body></html>"""


if __name__ == "__main__":
    main()
