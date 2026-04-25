---
name: gitnexus-impact-analysis
description: "Use when the user wants blast-radius analysis, safety checks before edits, or commit-time impact verification."
---

# GitNexus Impact Analysis

## Workflow

1. Read `gitnexus://repo/{name}/context`.
2. Run `gitnexus_impact({target: "symbolName", direction: "upstream"})` for every function, class, or method you plan to change.
3. If the result is HIGH or CRITICAL, warn the user before editing.
4. Use `gitnexus_context({name: "symbolName"})` to inspect the direct callers and processes behind d=1 results.
5. Before commit, review the staged diff directly and rerun `impact` for materially changed symbols. The current verified surface does not provide `detect_changes`.
6. After meaningful code changes, run `gitnexus-analyze-safe <repo_path>`.

## Checklist

- `impact` run before every symbol edit
- HIGH or CRITICAL risk surfaced to the user
- d=1 upstream dependents reviewed or updated
- staged diff reviewed directly before commit
- repo re-analyzed after meaningful code changes

## Example

```
gitnexus_impact({target: "voidInvoice", direction: "upstream"})
```

Use the result to identify which controllers, handlers, jobs, or UI actions must stay compatible with the change.
