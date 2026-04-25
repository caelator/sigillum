---
name: gitnexus-refactoring
description: "Use when the user wants to rename, extract, split, or restructure code safely."
---

# Refactoring with GitNexus

## Workflow

1. Read `gitnexus://repo/{name}/context`.
2. Run `gitnexus_context({name: "targetSymbol"})` to understand incoming and outgoing references.
3. Run `gitnexus_impact({target: "targetSymbol", direction: "upstream"})` before editing.
4. Make the refactor manually in code with targeted tests and staged diff review.
5. Re-run `impact` on the edited symbols before commit.
6. Run `gitnexus-analyze-safe <repo_path>` after the refactor lands.

## Important Constraint

Do not assume the current GitNexus surface exposes an automated `rename` tool. Verify first if you believe a newer version added it. Until then, use GitNexus for graph-guided analysis and do the code edits deliberately.

## Checklist

- `context` run on the target symbol
- `impact` run before edits
- callers and affected flows reviewed
- staged diff reviewed directly before commit
- repo re-analyzed after the refactor
