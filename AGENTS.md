<!-- gitnexus:start -->
# GitNexus + MemoryPort Standard

This repo is managed by the machine-wide GitNexus + MemoryPort standard.

## Tool Roles

- **GitNexus** is authoritative for code structure, dependency analysis, caller/callee context, execution-flow tracing, and repo indexing.
- **MemoryPort** is authoritative for durable cross-session memory such as decisions, preferences, runbooks, and sanitized work outcomes.
- **MemoryPort is not the source of truth for discoverable repo facts.** Use GitNexus and the code itself for those.

## GitNexus Workflow

### Session start

- Check `npx -y gitnexus@1.5.3 status` or read `gitnexus://repo/{name}/context` to confirm the index is fresh.
- If the index is stale, run `gitnexus-analyze-safe <repo_path>`.

### Exploration and debugging

- Use `gitnexus_query({query: "concept"})` to find relevant execution flows and symbols.
- Use `gitnexus_context({name: "symbolName"})` for callers, callees, and process participation.
- Use `gitnexus_impact({target: "symbolName", direction: "upstream"})` before changing any symbol.
- Use `gitnexus_cypher({query: "MATCH ..."})` only when the built-in tools are not enough.
- Read `gitnexus://repo/{name}/process/{processName}` when you need the full execution trace.

### Before editing symbols

- Run `gitnexus_impact` for each function, class, or method you plan to modify.
- Warn the user before proceeding when GitNexus reports HIGH or CRITICAL risk.
- Review and update all d=1 upstream dependents that would otherwise break.

### Before commit

- Review the staged diff directly.
- Re-run `gitnexus_impact` for materially changed symbols and verify the affected callers and flows still make sense.
- Do not rely on `detect_changes` or `rename` unless the installed GitNexus surface actually exposes them. The verified machine standard does not.

### After meaningful code changes

- Run `gitnexus-analyze-safe <repo_path>` so the index and generated repo docs stay in sync.

## MemoryPort Workflow

- At the beginning of work, query MemoryPort MCP for relevant prior decisions or project context.
- After durable outcomes, store a sanitized project-memory note through the MemoryPort MCP store tool.
- Prefer local retrieval first. Permanent-capable storage is used when a valid MemoryPort `uc_...` API key is configured.

## MemoryPort Safety Rules

- Never store secrets, API keys, tokens, passwords, or raw credentials.
- Never store unsanitized PHI, regulated medical data, or raw patient/customer/doctor records.
- Store durable summaries, decisions, preferences, environment notes, and sanitized outcomes instead.
- If something is sensitive but worth remembering, store only the minimal abstracted summary needed later.

## Skill Paths

| Task | Read this skill file |
|------|----------------------|
| Understand architecture / "How does X work?" | `.agents/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.agents/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.agents/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Refactor safely | `.agents/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.agents/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.agents/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
