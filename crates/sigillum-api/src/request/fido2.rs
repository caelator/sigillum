//! FIDO2 hardware key request contracts.

use serde::{Deserialize, Serialize};

/// Definition of a compartment during FIDO2 setup or addition.
///
/// `threshold` determines how many FIDO2 key taps are required to unlock this
/// compartment. `passphrase_mode` controls whether a passphrase fallback is
/// configured (e.g. "FIXED" for a setup-time passphrase).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompartmentDefinition {
    pub label: String,
    pub threshold: usize,
    pub passphrase_mode: Option<String>,
}

/// Initialize the vault with a FIDO2 hardware key and one or more compartments.
///
/// This is the primary setup path for new vaults. The first key registered
/// becomes the initial Shamir share holder. Compartment thresholds determine
/// how many distinct key taps are needed to unlock each compartment. `pin` is
/// optional so touch-only authenticators can be enrolled without forcing a PIN
/// round-trip; provide it only when the inserted key currently requires one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetupRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub label: String,
    pub compartments: Vec<CompartmentDefinition>,
    pub passphrase: Option<String>,
}

/// Register an additional FIDO2 key (or poison key) to the vault.
///
/// When `poison` is `true`, the key is registered as a decoy — tapping it
/// produces plausible deniability by appearing to unlock an empty vault.
/// `skip_keys` lists credential IDs of keys that should not participate
/// in the re-sharing ceremony (e.g. keys that are physically unavailable).
/// `pin` is optional and should only be supplied when the inserted key or the
/// re-sharing ceremony requires the current PIN.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RegisterRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub label: String,
    pub poison: Option<bool>,
    pub skip_keys: Option<Vec<String>>,
}

/// Unlock the vault by tapping one or more FIDO2 hardware keys.
///
/// `tap_count` specifies how many keys will be tapped in sequence.
/// Each key's HMAC-secret is used to decrypt its Shamir share; when enough
/// shares are gathered (meeting a compartment's threshold), that compartment
/// unlocks. Higher tap counts unlock higher-threshold compartments. `pins`
/// may be empty for touch-only authenticators; otherwise provide one PIN per
/// round or a single shared PIN for all rounds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2UnlockRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
    pub tap_count: usize,
}

/// Remove a FIDO2 key from the vault and re-share master keys among remaining keys.
///
/// `pin` is optional and should be provided only when the remaining enrolled
/// keys require their current PIN during the re-sharing ceremony.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2RemoveRequest {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    pub skip_keys: Option<Vec<String>>,
}

/// Set a brand-new FIDO2 PIN on an authenticator that does not have one yet.
///
/// This is intended for fresh hardware keys during setup or before registering
/// an additional backup key. Existing keys with a configured PIN should use
/// vendor tooling or a future dedicated change-PIN flow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fido2SetPinRequest {
    pub new_pin: String,
}
