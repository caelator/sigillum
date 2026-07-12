# Sigillum 1.0 Execution Runbook

**Status:** Active hardening and release handbook

**State recorded:** 2026-07-12, `main` at `815d262`

**Plan authority:** [release-1.0-plan.md](./release-1.0-plan.md)

The release plan defines product scope and acceptance criteria. This runbook
defines the current execution order and release mechanics. When they disagree
about product content, the plan wins. Do not treat old RC receipts as proof for
a commit that contains later hardening changes.

## 1. Current release truth

- The workspace and daemon UI declare version `1.0.0`.
- There is no final `v1.0.0` release tag or published final release.
- The `a22a98a` RC dry run proved the workflow shape at that historical commit;
  it is not release evidence for the current hardening line.
- The public `v1.0.0-rc.2` tag is a lightweight tag. The release workflow
  correctly rejected it because the contract requires an annotated tag, and no
  GitHub Release was published from it. Never reuse that RC number.
- A fresh RC is required after the capability-auth, payment-truth,
  queue-durability, no-HID, and release-governance fixes land.
- Gateway payments are preview-only and disabled by default. Opt-in balance
  observations are not finality proof and must not be represented as supported
  1.0 payment confirmations.

## 2. Execution order

1. Re-anchor on a clean, current `main`; confirm the GitNexus index matches the
   checked-out commit.
2. Complete stop-ship hardening in this order:
   - make generic daemon session authorization full-session-only and keep
     capability access explicit through scoped checks;
   - enforce payment amount truth, remove unsupported confirmation claims, and
     remove the unauthenticated privileged invoice-signing callback path;
   - persist exact signed transaction bytes (including the committed nonce) and
     hash as `prepared`, persist `submitted_unknown` before RPC, recover by
     receipt lookup or exact-byte resubmission without re-signing, and make the
     queue pause latch preemptive between jobs and immediately before broadcast;
   - compile, test, and lint `sigillum-fido2` with default features disabled;
   - harden the release tree, lockfile, workflow, and documentation contracts.
3. Run focused tests for every changed boundary, then run
   `./scripts/check-release.sh` alone. Never run a full gate while another agent
   or build is modifying the same checkout.
4. Review the complete diff, rerun GitNexus impact checks for materially changed
   symbols, and re-index the repository.
5. Land through a pull request and require both fixed-runner CI legs to pass.
6. Only then create a new annotated RC tag and complete the operator gates in
   section 5.

## 3. Executable release contract

`./scripts/check-release.sh` is the source release contract. It:

- resolves dependency-aware Cargo commands with the committed `Cargo.lock`;
- checks default and no-HID FIDO2 configurations;
- verifies UI lock metadata and generated assets;
- runs workspace, adversarial, runtime, browser, desktop, audit, and deny gates;
- exercises the queue's durable prepare/submission state machine and concurrent
  pause behavior; deterministic nonce or fee rejection must park for operator
  action rather than mint a replacement signature;
- fails if any check changes the tracked tree, even when the checkout started
  with an intentional local diff.

CI and release workflows use immutable action commits and explicit
`ubuntu-24.04` / `macos-15` runner lines. A release tag must:

- be annotated;
- equal `v<workspace-version>` or `v<workspace-version>-rc.N`;
- point to a commit on `origin/main` history;
- have a dated, non-empty matching `CHANGELOG.md` section.

The release workflow always creates a draft. Asset checksums, release notes,
and the operator decision are required before publication.

## 4. Failure handling

- Reproduce a failed step alone before classifying it as environmental.
- Treat compile errors, test failures, tracked-tree mutations, lockfile drift,
  malformed tags, and missing changelog sections as real release failures.
- For a new advisory, take a fixed dependency version first. Add a temporary
  ignore only when no fixed version exists, the affected code does not parse
  untrusted input, and both audit configuration and readiness documentation
  record an owner and removal condition.
- Use isolated `SIGILLUM_BASE_DIR` values and non-9743 ports for every smoke or
  test daemon. Never stop or reuse the operator's daemon.
- Preserve failure artifacts and exact receipts; do not convert a mock-provider
  result into a testnet or production claim.

## 5. Remaining operator gates

| Gate | Required evidence |
| --- | --- |
| F4 | Standard and chaos soak receipts on each supported host at the new RC SHA |
| F6 | Funded testnet receipts for the required execution families, with any mock-only family labeled honestly |
| Desktop | Checksum-verified RC `.dmg` installs and reaches unlock on a clean machine without a dev toolchain |
| Doctor | `sigillum doctor` passes on every supported host at the new RC |
| UI | Operator walkthrough/sign-off for the remaining C7 console acceptance surface |
| H2 | Explicit operator decision to tag and publish `v1.0.0` |

Work may continue on any independent item while one of these gates is waiting.
Do not mark H1 or H2 complete until every required receipt belongs to the same
release-candidate commit.

## 6. Fresh RC procedure

1. Fill the `CHANGELOG.md` 1.0.0 date and merge it with the hardening work.
2. From a clean `main`, run `./scripts/check-release.sh`.
3. Create and push an annotated `v1.0.0-rc.N` tag.
4. Require the release contract job, both verify legs, both artifact jobs, and
   the draft-release job to pass.
5. Download the draft assets and verify `SHA256SUMS`.
6. Complete the section 5 receipts against that exact RC SHA.
7. Delete the RC draft and tag after the rehearsal.
8. After explicit H2 approval, repeat the clean gate and push annotated
   `v1.0.0`; verify the draft before publishing it.

After publication, perform the post-release version/planning update described
by H3 in the plan of record.
