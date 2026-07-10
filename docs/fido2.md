# FIDO2

## What Exists Today

Sigillum supports local FIDO2/HID-based protection of compartment master keys.

The current implementation is designed around:

- local USB security keys
- the `hmac-secret` extension
- encrypted shard storage in `fido2_keys.json`
- compartment metadata stored separately as encrypted `meta.enc`

This is a local unlock flow. It is not a browser WebAuthn flow and not a remote authentication service.

The crate's `hid` feature is enabled by default. With
`default-features = false`, configuration, storage, and cryptographic helpers
remain available, but hardware-key operations are unavailable and PIN setup
returns an explicit disabled-feature error. The release gate verifies this
configuration independently.

## High-Level Model

When you register a key, Sigillum:

1. creates or uses compartment master keys
2. splits or associates recovery material for those compartments
3. encrypts shard blobs for storage
4. records registered-key metadata in `fido2_keys.json`

When you unlock with FIDO2, Sigillum:

1. talks to locally attached hardware keys over HID
2. asks for enough taps to satisfy the requested threshold(s)
3. reconstructs the needed compartment master keys
4. loads those keys into local memory

The result is one or more unlocked local compartments.

## Storage

The FIDO2 config file is:

```text
~/.sigillum/fido2_keys.json
```

Per-compartment encrypted metadata lives under:

```text
~/.sigillum/compartments/<id>/meta.enc
```

Optional passphrase wrapping data lives alongside each compartment when configured:

```text
~/.sigillum/compartments/<id>/passphrase.salt
~/.sigillum/compartments/<id>/passphrase_wrapped_key.enc
```

## CLI Surface

The implemented CLI surface includes:

```text
sigillum setup
sigillum unlock
sigillum fido2 status
sigillum fido2 list
sigillum fido2 register --label <LABEL>
sigillum fido2 remove --label <LABEL>
sigillum fido2 unlock
```

Exact behavior is still local-first and compartment-oriented. The daemon UI exposes the same general capabilities through local HTTP routes.

## Passphrase Interaction

Sigillum can also store a passphrase-wrapped version of a compartment master key. In the current product, that serves as a local fallback unlock path.

That passphrase support is:

- a local recovery and unlock mechanism
- not a vault snapshot feature
- not a remote recovery service

## Constraints

The current implementation should be understood with these limits:

- unlock happens through local HID access, not browser WebAuthn
- key state and unlocked compartments are local to the running process
- losing access to all valid recovery methods can permanently block access to Tier 2 data
- poison-key and deniability behavior exist in code, but they should be treated as advanced local features rather than polished product workflows

## Operational Advice

- register more than one real key if the protected data matters
- test both your FIDO2 and passphrase unlock paths on the machine you actually use
- use the daemon when you want persistent local unlock state across multiple management actions
