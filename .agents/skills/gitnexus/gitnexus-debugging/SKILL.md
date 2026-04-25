---
name: gitnexus-debugging
description: "Use when the user is debugging a bug, tracing an error, or asking why something fails."
---

# Debugging with GitNexus

## Workflow

1. Read `gitnexus://repo/{name}/context` and confirm the index is fresh.
2. Run `gitnexus_query({query: "<error, symptom, or behavior>"})` to find likely execution flows.
3. Run `gitnexus_context({name: "<suspect symbol>"})` for callers, callees, and related processes.
4. Read `gitnexus://repo/{name}/process/{processName}` for the full step-by-step trace.
5. Run `gitnexus_impact({target: "<symbol>", direction: "upstream"})` before editing the fix.
6. If this looks like a regression, review the git diff directly and rerun `impact` on the changed symbols. Do not assume `detect_changes` exists.

## Good Prompts

- "Where does this error come from?"
- "Why does this request fail?"
- "Trace how this doctor invoice action is handled."
- "What process writes this status?"

## Checklist

- Reproduce the symptom and capture the exact error.
- Use `query` to locate relevant flows.
- Use `context` to confirm the main symbols involved.
- Inspect the process resource when the order of calls matters.
- Run `impact` before changing anything.
- After the fix, rerun `gitnexus-analyze-safe <repo_path>` if the code changed materially.
