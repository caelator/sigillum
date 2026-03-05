//! FIDO2 HID operations using `ctap-hid-fido2`.
//!
//! All functions acquire a global lock to prevent concurrent HID access,
//! and are wrapped in a timeout to prevent indefinite blocking.

use std::sync::Mutex;

use crate::crypto::application_salt;
use crate::error::Fido2Error;

const FIDO_TIMEOUT_SECS: u64 = 60;
pub const RP_ID: &str = "sigillum.dev";

static FIDO_DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// Execute a blocking FIDO2 operation with timeout and global lock.
fn with_fido_timeout<F, T>(label: &str, f: F) -> Result<T, Fido2Error>
where
    F: FnOnce() -> Result<T, Fido2Error> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let label_owned = label.to_string();
    std::thread::spawn(move || {
        let _guard = FIDO_DEVICE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(FIDO_TIMEOUT_SECS))
        .map_err(|_| Fido2Error::Timeout {
            operation: label_owned,
            timeout_secs: FIDO_TIMEOUT_SECS,
        })?
}

/// Result of MakeCredential: credential ID and public key.
pub struct CredentialResult {
    pub credential_id: Vec<u8>,
    pub public_key_der: Vec<u8>,
    pub public_key_pem: String,
}

/// Count connected FIDO2 devices.
pub fn detect_devices() -> usize {
    ctap_hid_fido2::get_fidokey_devices().len()
}

/// Check if at least one FIDO2 device is present.
pub fn is_device_present() -> bool {
    detect_devices() > 0
}

/// Create a new credential on the FIDO2 device.
pub fn make_credential(pin: &str) -> Result<CredentialResult, Fido2Error> {
    let pin_owned = pin.to_string();

    with_fido_timeout("make_credential", move || {
        use ctap_hid_fido2::{fidokey::MakeCredentialArgsBuilder, verifier, Cfg, FidoKeyHidFactory};

        let devs = ctap_hid_fido2::get_fidokey_devices();
        let dev = devs.into_iter().next().ok_or(Fido2Error::NoDevice)?;
        let device = FidoKeyHidFactory::create_by_params(&[dev.param], &Cfg::init())
            .map_err(|e| Fido2Error::Other(format!("open device: {e}")))?;

        let challenge = verifier::create_challenge();
        let args = MakeCredentialArgsBuilder::new(RP_ID, &challenge)
            .pin(&pin_owned)
            .build();

        let att = device.make_credential_with_args(&args).map_err(|e| {
            let err = format!("{e}");
            if err.contains("0x01") || err.contains("CTAP1") {
                Fido2Error::Ctap1Device
            } else if err.contains("0x31") {
                Fido2Error::IncorrectPin
            } else {
                Fido2Error::Other(format!("make_credential: {e}"))
            }
        })?;

        let verify = verifier::verify_attestation(RP_ID, &challenge, &att);
        if !verify.is_success {
            return Err(Fido2Error::AttestationFailed);
        }

        Ok(CredentialResult {
            credential_id: verify.credential_id,
            public_key_der: verify.credential_public_key.der,
            public_key_pem: verify.credential_public_key.pem,
        })
    })
}

/// Get the hmac-secret output for a given credential.
/// The output is deterministic for the same credential + application salt.
pub fn get_hmac_secret(credential_id: &[u8], pin: &str) -> Result<[u8; 32], Fido2Error> {
    let cred_id_owned = credential_id.to_vec();
    let pin_owned = pin.to_string();
    let salt = application_salt();

    with_fido_timeout("get_hmac_secret", move || {
        use ctap_hid_fido2::{
            fidokey::{get_assertion::get_assertion_params::Extension, GetAssertionArgsBuilder},
            verifier, Cfg, FidoKeyHidFactory,
        };

        let devs = ctap_hid_fido2::get_fidokey_devices();
        let dev = devs.into_iter().next().ok_or(Fido2Error::NoDevice)?;
        let device = FidoKeyHidFactory::create_by_params(&[dev.param], &Cfg::init())
            .map_err(|e| Fido2Error::Other(format!("open device: {e}")))?;

        let challenge = verifier::create_challenge();
        let salt_hex = hex::encode(salt);
        let hmac_ext = Extension::create_hmac_secret_from_string(&salt_hex);

        let args = GetAssertionArgsBuilder::new(RP_ID, &challenge)
            .pin(&pin_owned)
            .credential_id(&cred_id_owned)
            .extensions(&[hmac_ext])
            .build();

        let assertions = device.get_assertion_with_args(&args).map_err(|e| {
            let err = format!("{e}");
            if err.contains("0x01") || err.contains("CTAP1") {
                Fido2Error::Ctap1Device
            } else if err.contains("0x31") {
                Fido2Error::IncorrectPin
            } else {
                Fido2Error::Other(format!("get_assertion: {e}"))
            }
        })?;

        if assertions.is_empty() {
            return Err(Fido2Error::NoHmacSecret);
        }

        for ext in &assertions[0].extensions {
            if let Extension::HmacSecret(Some(hmac)) = ext {
                return Ok(*hmac);
            }
        }

        Err(Fido2Error::NoHmacSecret)
    })
}
