# Sigillum 1.0 Execution Runbook

**Status:** Active hardening and release handbook

**State recorded:** 2026-07-31. `v1.0.0-rc.7` tag object
`c086f10ef411fc6341a713f3e98ba32e97351096` peels to protected-main commit
`3a4dbbf5294710056b2f0685b4c9bb9e985c730a`. Release run
`30612063470` passed all six jobs; its unique GitHub Release is an unpublished
`prerelease=true` draft with six nonempty assets, and the five payload assets
independently match `SHA256SUMS`. Preserve RC7 unchanged. The authorized
macOS 26.x/aarch64 support and operator-evidence contract changes `main`, so
RC7 is historical proof of the former macOS 15 contract and cannot be promoted
or reused for final release. The contract, workflows, checker/tests, and
current public release documentation must migrate through protected main and
required CI before RC8 is eligible. No final `v1.0.0` tag or published stable
release exists.

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
- `v1.0.0-rc.6` is an immutable failed-workflow receipt. Remote annotated tag
  object `1687443c67e6a90b1db84c78d6f372463dc8c639` peels to protected-main
  commit `194a90384bccef65bed42cf491d763a4c46948c0`. Release run `30600446396`
  passed the release contract, both source-verification legs, and both artifact
  jobs. `gh release create` then created the correct unique unpublished
  `prerelease=true` draft with all six expected nonempty assets, and an
  independent download verified every `SHA256SUMS` entry. The final job still
  failed because its paginated release-list query ran 336 ms after creation and
  did not yet observe the draft. A valid-looking draft does not override a red
  workflow receipt; preserve RC6 and its assets as historical evidence.
- `v1.0.0-rc.7` is an immutable successful historical draft. Remote annotated
  tag object `c086f10ef411fc6341a713f3e98ba32e97351096` peels to protected-main commit
  `3a4dbbf5294710056b2f0685b4c9bb9e985c730a`. Release run `30612063470`
  passed the release contract, both source-verification legs, both artifact
  jobs, and draft-release job. The unique unpublished `prerelease=true` draft
  has all six expected nonempty assets, and an independent download verified
  every `SHA256SUMS` entry. Tagged-source F7 migration tests passed on both
  verify legs.
- The authorized macOS 26.x/aarch64 support and operator-evidence contract
  supersedes RC7's macOS 15 contract. Do not move, delete, rerun, reuse, or
  promote RC7. The next eligible candidate is `v1.0.0-rc.8`, only after the
  executable contract migration passes the clean exact-HEAD gate and mandatory
  independent review, lands through protected main, and passes required CI.
  Rerun every H1 gate at the exact RC8 peeled SHA.
- Gateway payments are preview-only and disabled by default. Opt-in balance
  observations are not finality proof and must not be represented as supported
  1.0 payment confirmations.

## 2. Execution order

1. Implement the complete macOS 26.x/aarch64 contract migration: use explicit
   `macos-26` release and CI runners/contexts, require macOS 26.x/aarch64 in the
   release-evidence checker and its tests, and synchronize current public
   release documentation. Preserve all other release contracts.
2. Run `./scripts/check-release.sh` from a clean checkout at that exact
   integrated HEAD and complete the mandatory independent review. Never
   overlap a full gate with another agent or build modifying the checkout.
3. Live `main` protection was observed on 2026-07-31 with strict required
   checks exactly `rust (ubuntu-24.04)` and `rust (macos-26)` from GitHub App
   ID `15368`, `enforce_admins=true`, and force-push/deletion disabled.
   Release-tag governance must still prevent updates and deletion. Recheck this
   mutable external state after the migration lands and before tagging; then
   require both CI legs to have actually passed for the protected-main commit.
4. Only then create annotated `v1.0.0-rc.8`. Require its unique GitHub Release
   to remain draft, unpublished, and `prerelease=true`; verify its live asset
   checksums and complete every gate in section 5 at the RC8 peeled SHA.

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

RC7's historical CI and release workflows used immutable action commits and
explicit `ubuntu-24.04` / `macos-15` runner lines. RC8 is ineligible until the
workflows use immutable action commits with explicit `ubuntu-24.04` /
`macos-26` runner lines and expose the corresponding `rust (macos-26)` required
context. A release tag must:

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
> Exact-byte promotion and release-state enforcement are implemented, but they
> are not release evidence for the migrated line. H2 remains blocked until the
> complete macOS 26.x/aarch64 contract migration passes the clean exact-HEAD
> gate and independent review, lands through protected main with required CI,
> and RC8 satisfies F7, schema-v2
> same-host F4, funded F6, doctor, clean-install, C7, and evidence-bundle gates.

The release workflow always creates a draft. RC releases must remain
unpublished drafts with `prerelease=true`; the final draft and published
release must have `prerelease=false`.
`scripts/check-release-state-contract.sh` rejects any other RC-draft,
final-draft, or final-published state. RC tags build and checksum the candidate
payloads once. The final tag still runs both source-verification
legs, but its artifact jobs are intentionally skipped: the release job selects
the highest retained RC identified by the remote tag contract, requires its
single GitHub Release to remain an unpublished `prerelease=true` draft with the
exact six-asset shape, downloads and verifies its five payloads, copies those
exact bytes under final names, regenerates `SHA256SUMS`, and checks
byte-for-byte plus tag-normalized digest equality with
`scripts/promote-release-assets.sh`. It revalidates that exact RC tag object as
the live highest retained RC before promotion and immediately around draft
creation, so a newer same-commit RC cannot silently change the promotion
source. A final release must never substitute fresh rebuilds for the qualified
RC bytes. Release notes, verified assets, and the evidence bundle must be
complete before the operator records H2 approval and invokes the final-tag
ceremony. That approval authorizes conditional publication only if every
post-tag verification succeeds.

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

## 5. Remaining RC8 gates

| Gate | Required evidence |
| --- | --- |
| F4 | Schema-v2 standard 3600-second and chaos 600-second soak receipts on the same macOS 26.x/aarch64 host at the RC8 SHA; earlier RC receipts are historical only |
| F6 | Five funded public-testnet transactions at RC8 for four families, including both confirmed legs of `fund_gas` → dependent sweep; mock evidence cannot satisfy this gate |
| Desktop | Checksum-verified RC8 `.dmg` installs and reaches unlock on a clean machine without a dev toolchain |
| F7 | 0.1-era data-directory and snapshot upgrade verification passes in both tagged-source verify jobs at RC8 |
| Doctor | `sigillum doctor` passes on the eligible macOS 26.x/aarch64 host at RC8 |
| UI | Real-daemon browser smoke plus operator walkthrough/sign-off for all five destinations, palette, keyboard/focus, modal, and accessibility behavior at RC8 |
| H2 | Blocked until the exact RC8 commit and every automated and operator gate pass; explicit operator approval is then recorded immediately before the final-tag ceremony and authorizes conditional publication only after every post-tag verification passes |

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
   `release/asset-SHA256SUMS`. Both F4 soak receipts use schema v2 and record
   `platform: macos`, the exact `sw_vers -productVersion` value, canonical
   `aarch64`, and an opaque SHA-256 of the machine identity. The collector
   records the observed host without deciding eligibility; the bundle checker
   is the fail-closed qualification gate. It requires macOS 26.x on both soak
   receipts and requires their identity digests to match. The clean-install,
   doctor, and UI operator receipts also use schema v2 and share one exact RC
   object (`tag`, `tag_object`, and `peeled_sha`). They bind the qualified
   artifact filename and digest, the `mac-server` host identity (`macos`,
   macOS 26, `aarch64`, and the same opaque identity digest as both F4
   receipts), and the release operator identity and UTC review time. The
   clean-install receipt names the exact RC dmg, application version,
   identifier and install path, records checksum/dev-toolchain/unlock
   booleans, and checksum-binds `desktop/screenshots/unlock.png`. The doctor
   receipt names the exact RC macOS CLI archive, its installed executable hash
   and version, and structured all-ok checks. The C7 UI receipt names the exact
   RC dmg, all five destinations, the full H1 journey, setup/locked/unlocked
   states, and three fixed screenshot hashes. The asset checksum file is the
   independently verified five-entry `SHA256SUMS` body from the RC draft. F6
   audit exports live below `f6/audit/` and are referenced by
   `f6/receipts.json`.
2. Put `SHA256SUMS` inside the bundle, name the completed archive
   `sigillum-v1.0.0-release-evidence.tar.gz`, and compute its SHA-256. H2
   records that exact evidence filename and archive digest in the
   annotated final tag message, making the protected tag the immutable binding
   between the released code and its operator evidence.
   `scripts/check-release-evidence-bundle.sh` rejects missing, empty,
   unchecksummed, duplicate, unsafe, or linked archive members; requires the
   manifest to bind the exact RC tag-object ID and peeled SHA; rejects legacy
   F4 receipts and validates their same-host macOS 26.x/aarch64 identity plus
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
3. The executable H2 ceremony first downloads the qualified RC draft and
   requires its `SHA256SUMS` to equal the checksum body embedded in the
   validated evidence bundle. It then waits for the exact final workflow:
   release contract, both source-verification legs, and release must succeed,
   while both RC-only artifact jobs must be skipped. It independently verifies
   that the final draft's five renamed payloads are byte-identical and have the
   same tag-normalized digest manifest as that qualified RC snapshot, uploads
   the evidence archive as a seventh asset without replacing an existing
   asset, then re-fetches and repeats every byte/digest check across the seven
   live draft assets. The final draft must remain `prerelease=false`. The
   evidence SHA-256 must match the digest in the protected final tag in the
   last executable checks immediately before the H2-authorized conditional
   publication.
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
4. Require the release contract job, `verify (ubuntu-24.04)`,
   `verify (macos-26)`, both artifact jobs, and the draft-release job to pass.
   Fetch the unique RC release metadata and run
   `bash ./scripts/check-release-state-contract.sh rc-draft "$RC_TAG"` with
   that JSON on standard input; it must still be a draft, unpublished, and
   explicitly marked as `prerelease=true`.
5. Download the draft assets and verify `SHA256SUMS`.
6. Complete the section 5 receipts against that exact peeled RC commit SHA,
   independently verify every F6 transaction through the declared public RPC
   or explorer, and assemble the sanitized release evidence bundle described
   above.
7. Record the tag name, tag-object ID, peeled commit SHA, workflow run, asset
   checksum result, evidence filename, and evidence archive digest. Keep the
   annotated RC tag permanently and retain the RC draft/assets through
   final-draft verification.
8. Only after the complete macOS 26.x/aarch64 contract migration passes the
   clean exact-HEAD gate and independent review, lands through protected main
   with required CI, and the resulting RC8 passes every automated and operator
   gate, record explicit H2 approval,
   repeat the clean gate, and push annotated `v1.0.0` at the identical peeled
   commit as the receipt-bearing RC, with the evidence filename and digest in
   the tag message. If `main` has moved or any intervening commit is required,
   the receipts are void and a new
   monotonically numbered RC is required. The final workflow must skip artifact
   rebuilds and copy the qualified RC draft's exact five payload bytes under
   final names. Independently verify byte identity and tag-normalized digests
   against the evidence-bound RC `SHA256SUMS` with
   `scripts/promote-release-assets.sh`, upload and reverify the digest-bound
   evidence bundle as the seventh asset, require the final draft to remain
   `prerelease=false`, then publish. Only after final publication may the older
   RC draft be deleted.

After publication, perform the post-release version/planning update described
by H3 in the plan of record.
