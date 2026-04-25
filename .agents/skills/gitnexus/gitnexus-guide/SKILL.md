---
name: gitnexus-guide
description: "Use when the user asks about GitNexus itself, available tools, resources, or the supported workflow."
---

# GitNexus Guide

## Verified Current Surface

### MCP tools

- `query`
- `context`
- `impact`
- `cypher`
- `list_repos`

### Resources

- `gitnexus://repo/{name}/context`
- `gitnexus://repo/{name}/clusters`
- `gitnexus://repo/{name}/cluster/{clusterName}`
- `gitnexus://repo/{name}/processes`
- `gitnexus://repo/{name}/process/{processName}`
- `gitnexus://repo/{name}/schema`

## Standard Workflow

1. Read repo context first.
2. Use `query` to locate concepts and flows.
3. Use `context` to inspect one symbol deeply.
4. Use `impact` before editing.
5. Use `gitnexus-analyze-safe <repo_path>` after meaningful code changes.

## CLI Note

When multiple repos are indexed on the same machine, raw CLI `query`, `context`, and `impact` commands require `--repo <name>`.

## Unsupported-by-Default Assumptions

Do not assume the current GitNexus surface includes:

- `detect_changes`
- `rename`

If you think a newer version added them, verify with `npx -y gitnexus@1.5.3 --help` before writing instructions that depend on them.
