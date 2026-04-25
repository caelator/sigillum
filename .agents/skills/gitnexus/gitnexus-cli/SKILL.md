---
name: gitnexus-cli
description: "Use when the user needs GitNexus CLI workflows like analyze, status, clean, list, or wiki, or when a repo needs safe re-indexing."
---

# GitNexus CLI Commands

Use the machine wrapper commands first when they exist on this machine:

- `agent-tools-bootstrap [agent|all]`
- `gitnexus-analyze-safe <repo_path>`
- `gitnexus-mirror-sync <owner/repo>`

## Core GitNexus CLI

### analyze

```bash
npx -y gitnexus@1.5.3 analyze
```

Use raw `analyze` only when you intentionally do not want the machine normalizer. The standard workflow is `gitnexus-analyze-safe`, which runs analyze and then normalizes repo docs and skills.

### status

```bash
npx -y gitnexus@1.5.3 status
```

Check whether the repo is indexed and whether the index is stale.

### list

```bash
npx -y gitnexus@1.5.3 list
```

List indexed repos from `~/.gitnexus/registry.json`.

### clean

```bash
npx -y gitnexus@1.5.3 clean
```

Delete a broken or unwanted index.

### wiki

```bash
npx -y gitnexus@1.5.3 wiki
```

Generate documentation from the graph when an LLM-backed wiki is actually needed.

## Standard Machine Workflow

1. Run `npx -y gitnexus@1.5.3 status` or read `gitnexus://repo/{name}/context`.
2. If stale, run `gitnexus-analyze-safe <repo_path>`.
3. Use the GitNexus exploration, debugging, impact-analysis, or refactoring skills for the actual task.

## Notes

- The verified current GitNexus surface includes `query`, `context`, `impact`, `cypher`, and resources.
- The verified current surface does not expose `detect_changes` or `rename`.
- When multiple repos are indexed on the same machine, add `--repo <name>` to raw CLI `query`, `context`, and `impact` commands.
