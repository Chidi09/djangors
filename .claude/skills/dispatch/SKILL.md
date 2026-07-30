---
name: dispatch
description: Delegate real code changes to deepseek via the opencode CLI, monitor dispatches live without polling blindly, detect and recover from stalls/timeouts, and review every diff before committing. Use this skill whenever the user wants work "delegated to deepseek", asks to dispatch/run opencode, asks why a dispatch seems stuck, or when operating under a directive to keep deepseek doing the implementation while Claude reviews/validates/commits.
---

# Dispatch Skill (Rango / Djangors)

This project's standing rule: **deepseek does the implementation, Claude reviews, validates, and commits.** Claude does not hand-author feature code directly — see "When Claude is allowed to edit directly" below for the narrow exceptions. This skill is the mechanics of running that loop reliably against this Cargo workspace.

Adapted from a sibling project's `deepseek-control` skill; the dispatch mechanics (headless server, monitoring, stall detection, retry, review) are general and carried over as verified. The **hard rule in "Never `git checkout` a file mid-dispatch"** below is Rango-specific and is the single most important addition — it comes from a real incident in this repo, not a hypothetical.

## Prerequisites

- `opencode` CLI at `/root/.opencode/bin/opencode`, `agy` CLI at `/root/.local/bin/agy` — both confirmed installed in this environment.
- Confirm available deepseek models before assuming a model string is valid:
  ```bash
  /root/.opencode/bin/opencode models 2>&1 | grep -i deepseek
  ```
- **Model fallback chain** (carried from prior verified use of this same `opencode` install — reconfirm if a dispatch here behaves unexpectedly, since provider availability can drift):
  1. `opencode-go/deepseek-v4-flash` — start here.
  2. `opencode/deepseek-v4-flash-free` — swap only the `-m` flag if step 1 stalls with zero output for 60-80s+.
  3. `deepseek/deepseek-v4-flash` — last resort; has been seen silently broken (10+ minutes, zero writes, no launch-time error) in other sessions on this box. Try it, don't assume it's still broken, but don't be surprised if it is.
  4. `agy --model "GPT-OSS 120B (Medium)"` for small, tightly-specified single-item mechanical edits (one function, one file, exact code given) — confirmed fast (~3 min including its own build check) on comparably small tasks elsewhere. `--add-dir <absolute-repo-path>` is mandatory even when already `cd`'d into the repo, or it has zero file access.

If all deepseek models stall on the same prompt/scope, split the task smaller rather than keep cycling models.

## Step 1 — Start a headless server once per session

Don't use bare `opencode run ... --auto > file 2>&1` (no `--attach`, no `--format json`) — that buffers all output until exit, so you can't distinguish "slow" from "hung."

```bash
/root/.opencode/bin/opencode serve --port 4096 --hostname 127.0.0.1 > server.log 2>&1
```
Launch with `run_in_background: true`. Verify:
```bash
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:4096/   # expect 200
```
One server can host dispatches against multiple directories via `--dir` per call.

## Step 2 — Write a detailed instruction file per task

Never pass a short inline prompt beyond a trivial one-liner. Write full instructions to a scratch `.txt` file, in this order:

1. **What's needed**, in plain terms, with the *why*.
2. **"Read X, Y, Z in full first"** — every reference file, especially an existing correct pattern to mirror. This project's design docs (`docs/design/*.md`) are the authoritative spec for most slices — always point the dispatch at the specific doc for the phase/section it's implementing, not a paraphrase of it. Be exhaustive: name every file, say what to look for in each ("read `queryset.rs` for the reserved-word column-quoting pattern," not just "read `queryset.rs`").
3. **Explicit scope**: "Scope: ONLY these files/crates: [...]. Do not touch [adjacent crate/file also worth naming]." Highest-leverage line in the prompt — most drift is a dispatch touching more than intended. In a Cargo workspace this matters especially for shared files: workspace `Cargo.toml`, and any `lib.rs` that many features route through (e.g. `crates/djangors-admin/src/lib.rs` has been a multi-section convergence point before).
4. **The exact contract**: struct field names, template context keys, route paths, verbatim — don't let deepseek guess names that already exist elsewhere (e.g. a template's expected context fields). If a struct/context is large, paste the real field list rather than describing it.
5. **Anti-fabrication guardrails** where relevant (e.g. don't invent a plausible-but-wrong crate API — verify against the actual crate source or docs.rs, or leave a `// TODO: confirm` rather than guessing a signature).
6. **Verification commands to run itself**, this project's real ones:
   ```bash
   cargo fmt --all
   cargo build --workspace --all-targets
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
   For anything touching migrations or model metadata, also tell it to run the migration against a real dev DB and report the exact SQL, not just "tests pass."
7. **"Report the final diff"** — useful for a first pass, never a substitute for reading it yourself (Step 7).
8. **Order sections so a partial run is still usable**: for multi-section design docs (the `5.x.y` slices under `docs/design/`), tell it to implement in the doc's own order and stop cleanly at a section boundary if it runs out of time, rather than leaving one section half-done across several files.

## Step 3 — Dispatch

```bash
SP=/path/to/scratchpad
cd /root/dev/Rango
timeout 900 /root/.opencode/bin/opencode run "$(cat $SP/fix_X.txt)" \
  --attach http://127.0.0.1:4096 \
  --dir /root/dev/Rango \
  -m opencode-go/deepseek-v4-flash \
  --format json \
  --auto \
  > "$SP/fix_X_output.jsonl" 2>&1
echo "FIX_X_EXIT:$?"
```
Run as a single Bash call with `run_in_background: true`. `timeout 900` is a backstop for large multi-section dispatches (this repo's slices have legitimately run 90+ minutes) — the retry logic below is what actually catches stuck runs.

**The trailing `echo "FIX_X_EXIT:$?"` is the only reliable success signal.** The harness's own "completed (exit code 0)" notification reflects the wrapper bash script's exit code, not necessarily the opencode child's — a `kill -9` on the child can leave the wrapper's `echo` still reporting success. Always read the raw output file and check the `FIX_X_EXIT:N` line; `124` means the `timeout` fired.

## Step 4 — Monitor without polling blindly

Never manually re-run `wc -l`/`cat` on the output file in a loop. Use the Monitor tool with a bounded shell loop:

```bash
SP=/path/to/scratchpad
for i in 1 2 3 4 5 6; do
  sleep 10
  w=$(grep -c '"tool":"write"\|"tool":"edit"' "$SP/fix_X_output.jsonl" 2>/dev/null || echo 0)
  echo "t=${i}0s writes=$w"
  if [ "$w" -gt 0 ]; then
    echo "FIRST_WRITE_DETECTED"
    break
  fi
done
echo "check complete"
```
Pass as `command` to the Monitor tool (`timeout_ms: 70000`, `persistent: false`). Count `write`/`edit` events specifically — a `read` or `bash` event isn't progress. Deepseek reading several design docs and reference files before its first edit is normal and can take minutes on this project's larger slices, not a stall by itself.

## Step 5 — Stall detection and retry

For a targeted, few-file dispatch: zero `write`/`edit` events within 30-60s = treat as frozen, kill and retry.

For a large, multi-section dispatch (reads several design docs, spans crates) — this project's normal case for a `5.x.y` slice — watch liveness instead of a fixed clock:
```bash
ps -eo pid,etimes,time,stat,pcpu | grep -f <(pgrep -f "fix_X_output" 2>/dev/null)
```
`Sl` state + slowly climbing `TIME` = alive, just slow on API round-trips. Zero CPU growth across two checks = actually stuck.

**Killing:**
```bash
ps aux | grep "fix_X_output" | grep -v grep | awk '{print $2}' | xargs -r kill -9
```
Then verify nothing partial landed before retrying:
```bash
git status --short
git diff --stat   # actual line counts, not just filenames
```
A "clean" snapshot at kill-time doesn't guarantee nothing lands a moment later — a kill signal doesn't always take effect instantly. Re-verify with a full `cargo build --workspace` right before every commit if a kill happened anywhere upstream in a touched file's recent history.

If a dispatch stalls twice on the same scope, split it into smaller independent pieces (one dispatch per crate/section) rather than retrying a third time unchanged.

**Tell the user when this is costing real time** — a task stalling twice, or cumulative stall+retry time running long, gets a one-line heads-up in your next reply, not silent continued retrying.

## Step 6 — Never `git checkout`/`git restore`/`git reset` a file mid-dispatch

**This destroyed a real dispatch in this repo.** During `5.8.12`, a dispatch had made ~150 correct edits across `crates/djangors-admin/src/lib.rs` (structs, trait methods, handlers, tests for multiple design-doc sections) over ~90 minutes. Near the end, one `replace_file_content` call matched the wrong lines. The dispatch tried to undo *just that bad edit* with `git checkout crates/djangors-admin/src/lib.rs` — which reverts the **entire file** to the last commit, not the last edit, and has no undo (plain working-tree edits aren't in reflog or stash until staged/committed). It silently destroyed all ~150 edits and the dispatch died without recovering. A second dispatch (`5.8.12b`) had to reconstruct everything from the design doc, using the surviving templates/other files as the ground truth for what `lib.rs` needed to satisfy.

**The rule, put in every dispatch instruction file that does multi-step edits to a shared file:** if an edit tool call fails to match, the fix is to re-view the current state of the file and issue a new, narrowly-scoped corrected edit — never `git checkout <path>`, `git restore <path>`, or any `git reset` against a file with any uncommitted work. If genuinely unsure whether a file has uncommitted work worth losing, `git diff --stat <path>` first and look at the real line count, not just `git status`.

This applies doubly to convergence files multiple design-doc sections route through in the same session (large `lib.rs` files, `AdminSite`/`ModelAdminConfig` definitions) — exactly the shape of file this happened to before.

## Step 7 — Sequencing vs. parallel dispatches

Run concurrently whenever dispatches touch disjoint crates/files — this is the default. Never run two in parallel if both will touch the same file, especially a workspace-wide convergence point (a shared `lib.rs`, the workspace `Cargo.toml`, a widely-`pub use`'d module) — two independent opencode sessions each snapshot the file at their own start time and have no idea the other is editing it; whichever writes last silently clobbers the other. If two in-flight dispatches turn out to share a file, kill and re-sequence one immediately.

Three concurrent dispatches against one server is a reasonable ceiling before throughput drops and sessions start looking stalled from resource contention alone.

## Step 8 — Review every diff before committing, every time

Not optional. Checklist, in order:
1. `git diff <touched-file>` for **every** file the dispatch touched, not just the ones you expected — a dispatch asked to change one handler can quietly reshape an adjacent one in the same file.
2. `grep` the diff for `^-` lines and eyeball each deletion — every deletion should be explainable by what you asked for.
3. Run the real verification yourself, don't trust a dispatch's self-reported "tests pass":
   ```bash
   cargo fmt --all -- --check
   cargo build --workspace --all-targets
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
4. For migrations: apply against a real dev DB and confirm the schema, don't just eyeball the SQL.
5. Watch for guessed names when a struct/context was "too large to enumerate" in the prompt — deepseek fills gaps with plausible-but-wrong field names if not given the real list verbatim.
6. Only then `git add` + commit, with a message noting what was found/fixed during review, not just what was asked for.

## When Claude is allowed to edit directly

Delegation is the default. Direct edits are reserved for:

- A one-line fix to a bug introduced by a `sed`/mechanical transform Claude itself just ran.
- Reverting a small, clearly-scoped regression found during review (restoring one handler's behavior undone by an unrelated dispatch) — corrective, not new implementation.
- Pure, deterministic data transformation with a verified external source and zero judgment calls.
- After a dispatch has stalled twice on the exact same small, mechanical, well-understood change — but prefer splitting scope and retrying via deepseek once more first.

If genuinely unsure whether something crosses the line, keep it as a dispatch.
