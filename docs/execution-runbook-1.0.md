# Sigillum 1.0 Execution Runbook

**Status:** Active hardening and release handbook

**State recorded:** 2026-07-30. `v1.0.0-rc.5` peels to protected-main
commit `7e04743` and has an unpublished historical draft. It does not certify
the integrated hardening line. RC2–RC4 remain immutable failure receipts. No
RC6, final `v1.0.0` tag, or published stable release exists. The exact
integrated HEAD still needs the clean release gate, independent review,
protected merge, and required CI before RC6 is eligible.

**Plan authority:** [release-1.0-plan.md](./release-1.0-plan.md)

The release plan defines product scope and acceptance criteria. This runbook
defines the current execution order and release mechanics. When they disagree
about product content, the plan wins. Do not treat old RC receipts as proof for
a commit that contains later hardening changes.

## 1. Current release truth

- The workspace and daemon UI declare version `1.0.0`.
- There is no final `v1.0.0` release tag or published final release.
- The `a22a98a` RC dry run proved the workflow shape at that historical commit;
  it is not release evidence for the current hardening line. Its deleted
  `v1.0.0-rc.1` tag remains permanently burned and is the sole legacy gap
  permitted before the retained sequence that starts at `rc.2`; do not
  recreate it.
- The code-level capability-auth, payment-truth, queue-durability, no-HID, and
  release-tag hardening landed at `0a97c18` and passed the then-current source
  gate.
- `v1.0.0-rc.2` is an immutable failed-contract receipt, not a valid candidate.
  Its remote tag is annotated, but the tag-time workflow trusted a runner-local
  ref that checkout rewrote to the peeled commit. No draft or assets were
  created.
- `v1.0.0-rc.3` is an immutable failed-packaging receipt, not a valid
  candidate. Its workflow and checksums passed, but the macOS app had only a
  linker signature: strict bundle verification failed, `Info.plist` was not
  bound, resources were unsealed, and `_CodeSignature/CodeResources` was
  absent. The weak source check had only looked for a `Signature=` metadata
  line. RC3 assets and operator receipts cannot certify a final release.
- The signing remediation landed through protected `main` and passed the
  strengthened source gate at `f73b861`.
- `v1.0.0-rc.4` is an immutable failed-evidence-contract receipt. Its F6
  validator accepted any numeric non-Sepolia chain as the required L2, so a
  mainnet or arbitrary chain could satisfy the public-testnet claim; it also
  represented the two-transaction `fund_gas` → sweep chain with one hash. Its
  queue treated `sent` (broadcast, unconfirmed) as prerequisite success instead
  of requiring `confirmed`. No RC4 operator receipt can certify a final
  release.
  Historical release run `29230844456` completed all six jobs and uploaded the
  checksum, CLI, app, dmg, and notices assets to an unpublished draft; that
  proves the signing remediation, not the invalidated F6/runtime contract.
- `v1.0.0-rc.5` is a retained historical draft. Remote tag object
  `c726ba913ace7f5ca64987454b1352ffdd9c8f77` peels to protected-main commit
  `7e047438f6305ef1cedecdf4790e1b0e1d7e1e6e`. Release workflow
  `29248938476` passed all six jobs; its five payload assets independently
  match `SHA256SUMS`. Standard/chaos F4 and doctor receipts bind to the same
  SHA. The GitHub Release remains draft/unpublished.
- RC5 is incomplete and cannot certify the integrated hardening line. No
  qualifying F6 public-testnet receipt set, clean-install receipt, C7 sign-off,
  or complete external evidence bundle exists for the current line. The next
  eligible candidate is `v1.0.0-rc.6` only after exact-HEAD gating, review,
  protected merge, and CI; rerun every H1 gate at its exact peeled SHA.
- Gateway payments are preview-only and disabled by default. Opt-in balance
  observations are not finality proof and must not be represented as supported
  1.0 payment confirmations.

## 2. Execution order

1. Run `./scripts/check-release.sh` from a clean checkout at the exact
   integrated HEAD and complete the required independent review. Never overlap
   a full gate with another agent or build modifying the checkout.
2. Before merge, verify that `main` protection requires both fixed-runner CI
   legs, blocks force-pushes, and that release-tag governance prevents updates
   and deletion. Remediate missing settings before landing through a pull
   request, then require both CI legs to pass.
3. Only then create annotated `v1.0.0-rc.6`. Require its unique GitHub Release
   to remain draft, unpublished, and `prerelease=true`; verify its live asset
   checksums and complete every operator gate in section 5 at the RC6 peeled
   SHA.

## 3. Executable release contract

`./scripts/check-release.sh` is the source release contract. It:

- resolves dependency-aware Cargo commands with the committed `Cargo.lock`;
- checks default and no-HID FIDO2 configurations;
- verifies UI lock metadata and generated assets;
- runs the pinned 15-scenario axe-core accessibility gate; mock accessibility
  and screenshots do not replace real-daemon browser smoke;
- runs workspace, adversarial, runtime, browser, desktop, audit, and deny gates;
- on macOS, builds app/dmg bundles through the project signing wrapper, rejects
  incomplete or mixed Apple credentials, refuses Developer ID without one
  complete notarization family, explicitly notarizes/staples the dmg after
  Tauri creates it, and verifies both the source app and
  the app mounted read-only from the dmg with strict bundle, identifier,
  bound-plist, sealed-resource, signature-mode, stapled-ticket (Developer ID
  app copies and dmg), and CDHash checks; negative
  regressions reproduce the RC3 linker-only shape and malformed dmg layouts;
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
- have a dated, non-empty matching `CHANGELOG.md` section;
- for the final tag, carry exactly one `Release-Evidence-File` field naming
  `sigillum-v<workspace-version>-release-evidence.tar.gz` and exactly one
  `Release-Evidence-SHA256` field containing its lowercase SHA-256 digest.

`scripts/check-release-tag-contract.sh` enforces these rules against the
authoritative remote direct and peeled tag refs. It does not trust the
runner-local tag ref, because a tag-triggered checkout can rewrite that ref to
the peeled commit. If the tag object is absent locally, it fetches the remote
tag into a non-tag scratch ref and proves that the fetched object still matches
the authoritative observation. RC tags must also advance the retained remote
sequence by exactly one. For 1.0.0 only, the already-burned and historically
deleted `rc.1` is an explicit legacy exception; every tag from `rc.2` onward
must remain contiguous. The contract job pins the observed tag-object ID into
the final draft-release job. `scripts/test-release-tag-contract.sh` reproduces
the checkout rewrite and object-absent cases hermetically and proves that
lightweight, wrong-SHA, off-main, skipped-number, malformed,
changelog-invalid, and missing or malformed final-evidence tags fail closed.
The normal source release gate runs this regression test before tag time.

> [!IMPORTANT]
> The source gate above is executable now; the exact-byte final-tag path below
> is not. This merge-in-progress checkout still lacks the promotion script,
> final-tag workflow branches, and release-state enforcement. Do not invoke H2
> until that evidence-hardening implementation lands and the exact integrated
> HEAD repeats the clean gate and independent review.

The release workflow always creates a draft. RC releases must remain
unpublished drafts with `prerelease=true`; the final draft and published
release must have `prerelease=false`. RC tags build and checksum the candidate
payloads once. The final tag reruns the source-verification legs, but its
artifact jobs are intentionally skipped. The release job selects the qualified
RC, requires its exact unpublished-prerelease six-asset shape, downloads and
verifies its five payloads, copies those exact bytes under final names,
regenerates `SHA256SUMS`, and verifies byte-for-byte plus tag-normalized digest
equality. A final release must never substitute fresh rebuilds for the
qualified RC bytes. Release notes, verified assets, and the evidence bundle
must be complete before the operator records H2 approval and invokes the
final-tag ceremony. That approval authorizes conditional publication only if
all post-tag verification succeeds.

## 4. Failure handling

- Reproduce a failed step alone before classifying it as environmental.
- If checkout has rewritten a runner-local tag ref, diagnose annotation from
  the remote direct and `^{}` refs. Do not weaken the annotated-tag requirement
  or repair the ephemeral local ref as a substitute for remote truth.
- Never move, delete, or reuse any pushed RC tag, whether its workflow failed
  or passed. Preserve it as historical evidence, land fixes through the normal
  PR/gate path, and increment `rc.N`.
- A green artifact job is not sufficient when a shipped bundle fails strict
  signature verification. Preserve the draft, checksums, and failed app as
  evidence; do not promote its same-SHA soak, doctor, or install receipts.
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
| F4 | Schema-v2 standard 3600-second and chaos 600-second soak receipts on the same macOS 15.x/aarch64 host at the RC6 SHA; RC5 receipts are historical only |
| F6 | Five funded public-testnet transactions at RC6 for four families, including both confirmed legs of `fund_gas` → dependent sweep; mock evidence cannot satisfy this gate |
| Desktop | Checksum-verified RC6 `.dmg` installs and reaches unlock on a clean machine without a dev toolchain |
| F7 | 0.1-era data-directory and snapshot upgrade verification passes at RC6 |
| Doctor | `sigillum doctor` passes on the eligible macOS 15.x/aarch64 host at RC6 |
| UI | Real-daemon browser smoke plus operator walkthrough/sign-off for all five destinations, palette, keyboard/focus, modal, and accessibility behavior |
| H2 | Blocked until the evidence-hardening implementation is integrated and re-gated; then explicit operator approval is recorded immediately before the final-tag ceremony and authorizes conditional publication only after every post-tag verification passes |

Work may continue on any independent item while one of these gates is waiting.
Do not mark H1 or H2 complete until every required receipt names the same
release-candidate peeled commit SHA.

### Release evidence bundle

F4, F6, desktop, doctor, and UI evidence is generated only after the immutable
RC is created, so it cannot be committed into its own receipt-bearing SHA.
Preserve the sanitized evidence outside the checkout until final promotion:

1. Create one bundle containing a manifest, the RC tag-object ID and peeled
   commit SHA, release-workflow run, independently verified asset checksums,
   F4 receipts, F6 transaction evidence and audit exports, clean-install and
   doctor evidence, and UI sign-off. Exclude credentials, seeds, bearer tokens,
   private keys, and unsanitized customer or operator data.
   The fixed non-empty members are:
   `MANIFEST.json`, `SHA256SUMS`, `f4/standard.json`, `f4/chaos.json`,
   `f6/receipts.json`, `desktop/clean-install.json`,
   `doctor/mac-server.json`, `ui/signoff.json`, and
   `release/asset-SHA256SUMS`. Both F4 receipts use schema v2, record
   `platform: macos`, the exact macOS `ProductVersion`, canonical `aarch64`,
   and an opaque SHA-256 machine identity; the validator requires macOS 15.x
   and the same identity in both receipts. The doctor receipt is structured,
   bound to the RC SHA, and records the installed-RC pass. The asset checksum
   file is the independently verified five-entry `SHA256SUMS` body from the RC
   draft. F6 audit exports live below `f6/audit/` and
   are referenced by `f6/receipts.json`.
2. Put `SHA256SUMS` inside the bundle, name the completed archive
   `sigillum-v1.0.0-release-evidence.tar.gz`, and compute its SHA-256. H2
   records that exact evidence filename and archive digest in the
   annotated final tag message, making the protected tag the immutable binding
   between the released code and its operator evidence.
   `scripts/check-release-evidence-bundle.sh` rejects missing, empty,
   unchecksummed, duplicate, unsafe, or linked archive members; requires the
   manifest to bind the exact RC tag-object ID and peeled SHA; rejects legacy
   F4 receipts and validates their same-host macOS 15.x/aarch64 identity plus
   configured and actual soak durations; requires Ethereum Sepolia (`11155111`)
   plus Base Sepolia (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia
   (`11155420`), and rejects legacy F6 schema v1. F6 schema v2 validates four
   families with five unique transaction hashes and audit exports. Its nested
   gas-top-up claim requires ordered `fund_gas` and dependent-sweep legs with
   matching plan/network/chain identities, distinct job and step IDs, confirmed
   successful receipts, top-up destination equal to sweep source, the sweep's
   prerequisites equal to the single top-up job, and a strictly later sweep
   broadcast timestamp and receipt block.
   It also binds every audit export to its transaction and the RC SHA, validates
   clean install and UI sign-off, and computes the outer archive digest. This
   is an offline structural check; it does not query a public RPC or explorer.
   Before H2, independently verify all five F6 transactions on the claimed
   public chains, including chain ID, successful receipt, finality, and the
   family effect represented by each audit export.
3. The executable H2 ceremony snapshots the qualified RC draft, then waits for
   the exact final workflow. Its contract, both source-verification legs, and
   release job must pass while both artifact jobs are skipped. It verifies the
   final draft's five renamed payloads are byte-identical and have the same
   tag-normalized digest manifest as the RC snapshot, uploads the evidence
   archive as a seventh asset without replacement, then re-fetches and repeats
   every byte/digest check across all seven live draft assets. The final draft
   must remain `prerelease=false`. The evidence SHA-256 must match the digest
   in the protected final tag before the H2-authorized conditional publication.
4. In H3, update `docs/production-readiness-audit.md` with the public release
   URL, evidence filename and digest, final tag-object ID, RC peeled SHA, and a
   sanitized receipt summary. This post-release documentation commit does not
   alter the released `v1.0.0` commit.

If the bundle changes after the final tag is created, do not publish. The final
tag is immutable, so restore the exact digest-bound bundle rather than moving
or recreating the tag.

## 6. Fresh RC procedure

1. Confirm the dated, non-empty `CHANGELOG.md` 1.0.0 section is on `main`.
2. Fetch `origin`, start from a clean `main`, record
   `GATE_SHA=$(git rev-parse HEAD)`, assert it equals `origin/main`, and run
   `./scripts/check-release.sh`.
3. Query `git ls-remote --tags --refs origin 'refs/tags/v1.0.0-rc.*'` and require
   the new `N` to equal the highest retained remote RC number plus one. Reassert
   `HEAD == GATE_SHA == origin/main`, then create and push the annotated tag.
   Every pushed `N` is permanently burned, even when no draft was created.
4. Require the release contract job, both verify legs, both artifact jobs, and
   the draft-release job to pass. Require the unique RC release to remain
   draft, unpublished, and `prerelease=true`.
5. Download the draft assets and verify `SHA256SUMS`.
6. Complete the section 5 receipts against that exact peeled RC commit SHA,
   independently verify every F6 transaction through the declared public RPC
   or explorer, and assemble the sanitized release evidence bundle described
   above.
7. Record the tag name, tag-object ID, peeled commit SHA, workflow run, asset
   checksum result, evidence filename, and evidence archive digest. Keep the
   annotated RC tag permanently and retain the RC draft/assets through
   final-draft verification.
8. Only after the evidence-hardening prerequisite is integrated, re-gated, and
   reviewed, record explicit H2 approval, repeat the clean gate, and push annotated
   `v1.0.0` at the identical peeled commit as the receipt-bearing RC, with the
   evidence filename and digest in the tag message. If `main` has moved or any
   intervening commit is required, the receipts are void and a new
   monotonically numbered RC is required. The final workflow must skip artifact
   rebuilds and copy the qualified RC draft's exact five payload bytes under
   final names. Verify byte identity and tag-normalized digests, regenerate
   `SHA256SUMS`, require the final draft to remain `prerelease=false`, upload
   and reverify the evidence bundle as the seventh asset, then publish. Only
   after final publication may the older RC draft be deleted.

After publication, perform the post-release version/planning update described
by H3 in the plan of record.
