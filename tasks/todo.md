# Daily Bug Scan 2026-04-01

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since `2026-03-31T14:00:25.004Z` and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `0eba91d480598f37601fc92ae83a7e59bb7a5ddd` from `2026-03-29T08:57:05-05:00`, which was already covered by prior runs
- [x] Skip fixes because there is no new committed diff, failing test, or CI signal in scope

# Daily Bug Scan 2026-03-31

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since `2026-03-30T14:00:23.276Z` and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `0eba91d480598f37601fc92ae83a7e59bb7a5ddd` from `2026-03-29T08:57:05-05:00`, which was already covered by the prior run
- [x] Skip fixes because there is no new committed diff, failing test, or CI signal in scope

# Daily Bug Scan 2026-03-30

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since `2026-03-29T14:00:11.031Z` and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `0eba91d480598f37601fc92ae83a7e59bb7a5ddd` from `2026-03-29T08:57:05-05:00`, which was already covered by the prior run
- [x] Skip fixes because there is no new committed diff, failing test, or CI signal in scope

# Daily Bug Scan 2026-03-29

- [x] Check automation memory and repo state before scanning recent history
- [x] Review the only in-scope commit `0eba91d480598f37601fc92ae83a7e59bb7a5ddd` from `2026-03-29T08:57:05-05:00`
- [x] Inspect the touched FIDO2 optional-PIN and `eth_xpub_export` paths for concrete regressions
- [x] Verify the new coverage with `cargo test -p sigillum-api fido2_ -- --nocapture`, `cargo test -p sigillum-daemon xpub_export_ -- --nocapture`, `cargo test -p sigillum-daemon pin_required_error_is_actionable -- --nocapture`, `cargo test -p sigillum-daemon raw_pin_required_error_is_normalized -- --nocapture`, and `cargo test -p sigillum-fido2 classifies_pin_required_errors -- --nocapture`
- [x] Skip fixes because the in-scope diff has no failing test, CI signal, or reproducible regression in evidence

# Daily Bug Scan 2026-03-28

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since `2026-03-27T14:00:04.569Z` and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `62af2fdcd6e0bd12c86f72f657a006eb19e8aa6c` from `2026-03-24T17:30:23-05:00`
- [x] Skip fixes because there is no concrete committed diff, failing test, or CI signal in scope

# Daily Bug Scan 2026-03-27 12:20Z

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since `2026-03-27T12:20:33.830Z` and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `62af2fdcd6e0bd12c86f72f657a006eb19e8aa6c` from `2026-03-24T17:30:23-05:00`
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Daily Bug Scan 2026-03-27

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-25T14:00:07.082Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `62af2fdcd6e0bd12c86f72f657a006eb19e8aa6c` from `2026-03-24T17:30:23-05:00`
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Daily Bug Scan 2026-03-25

- [x] Check automation memory and repo state before scanning recent history
- [x] Review the only in-scope commit `62af2fdcd6e0bd12c86f72f657a006eb19e8aa6c` from 2026-03-24 17:30:23 -0500
- [x] Confirm the new `eth_xpub_export` path allowed any valid session token to export a wallet profile from a different unlocked compartment
- [x] Bind xpub export to the caller's active compartment and add service-level regression coverage
- [x] Verify with `cargo test -p sigillum-daemon xpub_export_ -- --nocapture` and `cargo test -p sigillum-core ethereum_xpub -- --nocapture`

# FIDO2 Touch-Only Flow 2026-03-24

- [x] Audit the current FIDO2 request, service, HID, CLI, and UI layers for mandatory PIN assumptions
- [x] Make hardware-key PINs optional end to end so trusted-key touch is sufficient when the authenticator allows it
- [x] Refresh FIDO2 copy, prompts, and regression coverage to describe PINs as optional and verify the no-PIN path

# UX + Xpub Vertical Slice 2026-03-24

- [x] Reframe the embedded daemon UI into distinct, task-oriented operator modes instead of anchor-scroll cards
- [x] Add explicit in-product guidance that explains what Sigillum is, what is currently supported, and how to run the main local workflows
- [x] Implement an initial `eth-xpub` wallet family slice across core derivation, daemon/API/client contracts, profile storage, and embedded UI
- [x] Verify the redesigned UI and new xpub slice with targeted tests plus a real local daemon smoke pass

# Local Deployment Verification 2026-03-24

- [x] Confirm the local-only deployment target and capture the daemon/UI startup path for this verification pass
- [x] Start the local Sigillum daemon with an isolated local data directory and verify it is serving on loopback
- [x] Exercise a local health/UI smoke path and collect evidence from HTTP responses and process logs
- [x] Run targeted verification commands that support the local deployment confidence level and record the results

# Release Integration 2026-03-24

- [x] Rebuild context for the remaining dirty worktree and confirm the intended release story against Sigillum's local-only north star
- [x] Audit the expanded daemon, client, API, gateway, and wallet/deposit changes for integration gaps or scope leaks
- [x] Fix the remaining implementation, test, and documentation issues with the smallest reasonable blast radius
- [x] Run the release verification gates and capture evidence before marking the change done
- [x] Commit, push, merge to `main`, and leave the workspace clean

# UI/UX Rebuild

- [x] Audit the current embedded daemon UI structure, state model, and product positioning
- [x] Install and read the requested `ui-ux-pro-max` skill, then derive a Sigillum-specific design direction from it
- [x] Rebuild the embedded UI shell, visual system, and information hierarchy around Sigillum's trust-first local operator workflow
- [x] Refresh the setup, locked, and unlocked states so the next action is clearer without changing daemon routes
- [x] Run formatting and targeted verification for the rebuilt embedded UI
- [x] Remove the dominant Workspace Map rail and rebalance the shell toward a quieter, more elegant in-flow navigation model
- [x] Reproduce and fix the local-session unlock flow regression in the embedded daemon UI
- [x] Reproduce and fix FIDO2 key registration behavior when multiple authenticators are inserted at once
- [x] Rebuild the daemon UI into a more minimalist, intuitive, and materially deep control-room experience

# Daemon Follow-Up

- [x] Audit the existing embedded daemon UI structure, state model, and first-run flow
- [x] Redesign the interface shell for first-run, locked, and unlocked states without changing daemon routes
- [x] Add stronger copy, guidance, empty states, and recommended-next-step affordances across the workspace
- [x] Run formatting and daemon tests
- [x] Restart the real local daemon and verify the rebuilt UI live in the browser
- [x] Classify CTAP PIN lockout errors in the FIDO2 layer
- [x] Surface actionable recovery guidance in the daemon setup and unlock UI
- [x] Audit the FIDO2 setup and unlock error path for raw CTAP lockout errors
- [x] Normalize FIDO2 hardware errors into actionable recovery guidance in the service layer and UI
- [x] Verify the updated local daemon and retest the recovery messaging path

# FIDO2 PIN Flow Design

- [x] Inspect the current FIDO2 request and response types for native PIN setup support
- [x] Inspect the daemon FIDO2 routes and service layer seams for a set-PIN endpoint
- [x] Inspect the setup wizard and hardware-key management UI placement for a native set-PIN flow
- [x] Summarize the cleanest integration points and likely validation and UX edge cases

# Native FIDO2 PIN Setup

- [x] Add a dedicated daemon API flow for setting a first FIDO2 PIN on a new hardware key
- [x] Reuse the HID layer and service mapping to normalize first-run PIN setup errors
- [x] Add in-app PIN setup affordances to both first-run setup and backup-key registration
- [x] Cover the new route, API types, and rendered UI entry points with regression tests
- [x] Re-run targeted crates plus the full workspace test suite before promoting the live daemon

# Session Reauth Recovery

- [x] Trace the locked-vs-already-unlocked mismatch to the daemon/session boundary
- [x] Allow unlock routes to re-authenticate a browser tab when the daemon is still unlocked but the session token is gone
- [x] Keep duplicate unlock attempts conflicting only when the caller already has a valid session
- [x] Add regression coverage for session-revoke followed by passphrase re-auth

# Multi-Key Onboarding

- [x] Audit the setup wizard for presets whose thresholds imply more enrolled keys than the flow currently collects
- [x] Add an explicit backup-key enrollment step after first-key setup for multi-key presets
- [x] Reuse the in-wizard PIN setup path for additional fresh hardware keys
- [x] Allow users to finish for now with a clear warning when they have not yet enrolled enough keys for higher-threshold compartments
- [x] Cover the new wizard entry points with rendered-HTML regression checks

# Setup Recovery

- [x] Audit the current setup and locked-state recovery affordances for failed or interrupted onboarding
- [x] Add a typed-confirmation daemon reset route that clears local Sigillum data and returns the daemon to first-run state
- [x] Surface snapshot-restore and reset controls directly in the setup and locked-session UI states
- [x] Cover both partial uninitialized state and initialized local-state reset with daemon integration tests
- [x] Rebuild the real local daemon and verify the live recovery controls are served by the actual CLI binary

# Daily Bug Scan 2026-03-19

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-18T15:25:09Z and the 24-hour fallback window
- [x] Confirm there are no commits in scope, so there is no concrete bug evidence to act on

# Daily Bug Scan 2026-03-20

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-19T14:00:06.367Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `32be0a7324246d2de14f4f9bcd5b6f957d8cfc0e` from 2026-03-05 07:41:01 -0500
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Daily Bug Scan 2026-03-21

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-20T14:00:28.528Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `32be0a7324246d2de14f4f9bcd5b6f957d8cfc0e` from 2026-03-05 07:41:01 -0500
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Daily Bug Scan 2026-03-22

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-21T14:00:16.150Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `32be0a7324246d2de14f4f9bcd5b6f957d8cfc0e` from 2026-03-05 07:41:01 -0500
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Daily Bug Scan 2026-03-23

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-22T14:00:02.746Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `32be0a7324246d2de14f4f9bcd5b6f957d8cfc0e` from 2026-03-05T07:41:01-05:00
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope

# Production Readiness Review 2026-03-21

- [x] Inspect the repo docs, task artifacts, and current workspace scope to identify Sigillum's intended production boundary
- [x] Review implementation and configuration for production blockers across the local daemon, gateway, and persistence paths
- [x] Run the workspace verification gates and summarize whether the current tree is actually release-ready

# Production Blocker Resolution 2026-03-21

- [x] Rework gateway payment creation so the tracked daemon deposit is the single source of truth for returned payment addresses
- [x] Add compensating cleanup when gateway persistence fails after daemon-side deposit creation
- [x] Persist webhook retry attempt counts correctly and cover the retry ledger with regression tests
- [x] Update the vulnerable dependency state and rerun tests, fmt, clippy, and audit

# Full Repository Audit 2026-03-22

- [x] Rebuild repo context from scratch and restate Sigillum's intended deployment boundary from docs and code
- [x] Inspect the codebase comprehensively across crates, interfaces, storage, auth, crypto, gateway, daemon, and tests
- [x] Run the release and security gates and deliver a prioritized audit for readiness, security, and production standards

# Audit Remediation 2026-03-22

- [x] Turn the audit findings into concrete code, test, and documentation changes with minimal blast radius
- [x] Add bounded outbound HTTP behavior and durable atomic-write semantics across daemon, client, and persistence paths
- [x] Harden gateway webhook delivery against post-validation SSRF drift and expand startup recovery beyond passive pending-operation counting
- [x] Remove inline-handler CSP exceptions from the embedded daemon UI and update security/readiness docs to match the actual implementation
- [x] Re-run workspace tests, fmt, clippy, and audit after the remediation pass

# Daily Bug Scan 2026-03-24

- [x] Check automation memory and repo state before scanning recent history
- [x] Verify commits since 2026-03-23T14:00:21.252Z and the 24-hour fallback window
- [x] Confirm both windows are empty and that the latest reachable commit is `32be0a7324246d2de14f4f9bcd5b6f957d8cfc0e` from `2026-03-05T07:41:01-05:00`
- [x] Skip fixes because there is no concrete commit, diff, test, or CI evidence in scope
