---
name: gitnexus-exploring
description: "Use when the user asks how code works, wants architecture context, or needs execution-flow discovery."
---

# Exploring with GitNexus

## Workflow

1. Start with `gitnexus://repo/{name}/context`.
2. Use `gitnexus_query({query: "concept"})` to find the most relevant process groups and symbols.
3. Use `gitnexus_context({name: "symbolName"})` to understand direct callers, callees, and participating processes.
4. Read `gitnexus://repo/{name}/process/{processName}` when you need the end-to-end flow rather than one symbol.
5. Use `gitnexus_cypher` only for questions that need raw graph queries.

## Good Prompts

- "How does invoice voiding work?"
- "Show me the clinic billing flow."
- "What calls this method?"
- "Which process updates doctor balances?"

## Checklist

- Confirm the index is fresh before trusting the graph.
- Prefer `query` over grep when the user is asking about behavior or concepts.
- Use `context` before opening files blindly.
- When you do open code, target the specific symbols GitNexus already identified.
