//! FIDO2 HID operations using `ctap-hid-fido2`.
//!
//! All functions acquire a global lock to prevent concurrent HID access,
//! and are wrapped in a timeout to prevent indefinite blocking.

use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::crypto::application_salt;
use crate::error::Fido2Error;

const FIDO_TIMEOUT_SECS: u64 = 60;
const FIDO_DEVICE_LOCK_POISONED: &str = "FIDO device access state is inconsistent after a worker panic; restart Sigillum before retrying";
pub const RP_ID: &str = "sigillum.dev";

static FIDO_DEVICE_LOCK: Mutex<()> = Mutex::new(());

fn classify_ctap_error(operation: &str, err: &str) -> Fido2Error {
    if err.contains("0x01") || err.contains("CTAP1") {
        Fido2Error::Ctap1Device
    } else if err.contains("0x2E") || err.contains("NO_CREDENTIALS") {
        Fido2Error::NoMatchingCredential
    } else if err.contains("0x36") || err.contains("PIN_REQUIRED") {
        Fido2Error::PinRequired
    } else if err.contains("0x35") || err.contains("PIN_NOT_SET") {
        Fido2Error::PinNotSet
    } else if err.contains("0x34") || err.contains("PIN_AUTH_BLOCKED") {
        Fido2Error::PinAuthBlocked
    } else if err.contains("0x33") || err.contains("PIN_AUTH_INVALID") {
        Fido2Error::IncorrectPin
    } else if err.contains("0x32") || err.contains("PIN_BLOCKED") {
        Fido2Error::PinBlocked
    } else if err.contains("0x31") || err.contains("PIN_INVALID") {
        Fido2Error::IncorrectPin
    } else {
        Fido2Error::Other(format!("{operation}: {err}"))
    }
}

fn normalize_pin(pin: Option<&str>) -> Option<&str> {
    pin.filter(|pin| !pin.is_empty())
}

/// Execute a blocking FIDO2 operation with timeout and global lock.
fn with_fido_timeout<F, T>(label: &str, f: F) -> Result<T, Fido2Error>
where
    F: FnOnce() -> Result<T, Fido2Error> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let label_owned = label.to_string();
    std::thread::spawn(move || {
        let result = match FIDO_DEVICE_LOCK.lock() {
            Ok(_guard) => f(),
            Err(_) => Err(Fido2Error::Other(FIDO_DEVICE_LOCK_POISONED.into())),
        };
        let _ = tx.send(result);
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

/// Result of creating a new credential and immediately deriving its hmac-secret.
pub struct CredentialEnrollmentResult {
    pub credential: CredentialResult,
    pub hmac_secret: [u8; 32],
}

/// Count connected FIDO2 devices.
pub fn detect_devices() -> usize {
    ctap_hid_fido2::get_fidokey_devices().len()
}

/// Check if at least one FIDO2 device is present.
pub fn is_device_present() -> bool {
    detect_devices() > 0
}

fn open_device(param: &ctap_hid_fido2::HidParam) -> Result<ctap_hid_fido2::FidoKeyHid, Fido2Error> {
    use ctap_hid_fido2::{Cfg, FidoKeyHidFactory};

    FidoKeyHidFactory::create_by_params(std::slice::from_ref(param), &Cfg::init())
        .map_err(|e| Fido2Error::Other(format!("open device: {e}")))
}

fn get_single_attached_device() -> Result<ctap_hid_fido2::HidParam, Fido2Error> {
    let devices = ctap_hid_fido2::get_fidokey_devices();
    match devices.as_slice() {
        [] => Err(Fido2Error::NoDevice),
        [device] => Ok(device.param.clone()),
        _ => Err(Fido2Error::MultipleDevicesDetected {
            count: devices.len(),
        }),
    }
}

fn make_credential_with_device(
    device: &ctap_hid_fido2::FidoKeyHid,
    pin: Option<&str>,
) -> Result<CredentialResult, Fido2Error> {
    use ctap_hid_fido2::{fidokey::MakeCredentialArgsBuilder, verifier};

    let challenge = verifier::create_challenge();
    let builder = MakeCredentialArgsBuilder::new(RP_ID, &challenge);
    let args = match normalize_pin(pin) {
        Some(pin) => builder.pin(pin).build(),
        None => builder.without_pin_and_uv().build(),
    };

    let att = device.make_credential_with_args(&args).map_err(|e| {
        let err = format!("{e}");
        classify_ctap_error("make_credential", &err)
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
}

fn get_hmac_secret_with_device(
    device: &ctap_hid_fido2::FidoKeyHid,
    credential_id: &[u8],
    pin: Option<&str>,
) -> Result<[u8; 32], Fido2Error> {
    use ctap_hid_fido2::{
        fidokey::{GetAssertionArgsBuilder, get_assertion::get_assertion_params::Extension},
        verifier,
    };

    let challenge = verifier::create_challenge();
    let salt_hex = hex::encode(application_salt());
    let hmac_ext = Extension::create_hmac_secret_from_string(&salt_hex);

    let builder = GetAssertionArgsBuilder::new(RP_ID, &challenge)
        .credential_id(credential_id)
        .extensions(&[hmac_ext]);
    let args = match normalize_pin(pin) {
        Some(pin) => builder.pin(pin).build(),
        None => builder.without_pin_and_uv().build(),
    };

    let assertions = device.get_assertion_with_args(&args).map_err(|e| {
        let err = format!("{e}");
        classify_ctap_error("get_assertion", &err)
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
}

fn select_registration_device(
    existing_credential_ids: &[Vec<u8>],
    pin: Option<&str>,
) -> Result<ctap_hid_fido2::HidParam, Fido2Error> {
    let devices = ctap_hid_fido2::get_fidokey_devices();
    if devices.is_empty() {
        return Err(Fido2Error::NoDevice);
    }

    if existing_credential_ids.is_empty() {
        return match devices.as_slice() {
            [device] => Ok(device.param.clone()),
            _ => Err(Fido2Error::MultipleDevicesDetected {
                count: devices.len(),
            }),
        };
    }

    let mut selected = None;
    for device_info in &devices {
        let device = open_device(&device_info.param)?;
        let mut matches_registered = false;
        for credential_id in existing_credential_ids {
            match get_hmac_secret_with_device(&device, credential_id, pin) {
                Ok(_) => {
                    matches_registered = true;
                    break;
                }
                Err(Fido2Error::NoMatchingCredential) => {}
                Err(error) => return Err(error),
            }
        }

        if !matches_registered {
            if selected.is_some() {
                return Err(Fido2Error::MultipleDevicesDetected {
                    count: devices.len(),
                });
            }
            selected = Some(device_info.param.clone());
        }
    }

    selected.ok_or(Fido2Error::NoNewDeviceDetected)
}

/// Set a brand-new FIDO2 PIN on the connected authenticator.
#[must_use = "check the Result for FIDO2 PIN setup errors"]
pub fn set_new_pin(pin: &str) -> Result<(), Fido2Error> {
    if pin.len() < 4 {
        return Err(Fido2Error::Other(
            "set_new_pin: new FIDO2 PIN must be at least 4 characters long".into(),
        ));
    }

    let pin_owned = Zeroizing::new(pin.to_string());

    with_fido_timeout("set_new_pin", move || {
        use ctap_hid_fido2::fidokey::get_info::InfoOption;

        let device = open_device(&get_single_attached_device()?)?;

        match device
            .enable_info_option(&InfoOption::ClientPin)
            .map_err(|e| Fido2Error::Other(format!("get_info: {e}")))?
        {
            Some(true) => return Err(Fido2Error::PinAlreadySet),
            Some(false) => {}
            None => {
                return Err(Fido2Error::Other(
                    "set_new_pin: this hardware key does not advertise clientPin support".into(),
                ));
            }
        }

        device.set_new_pin(&pin_owned).map_err(|e| {
            let err = format!("{e}");
            classify_ctap_error("set_new_pin", &err)
        })?;

        Ok(())
    })
}

/// Create a new credential on the FIDO2 device.
#[must_use = "check the Result for FIDO2 credential creation errors"]
pub fn make_credential(pin: Option<&str>) -> Result<CredentialResult, Fido2Error> {
    Ok(make_credential_with_hmac(pin, &[])?.credential)
}

/// Create a new credential on the chosen device and derive its hmac-secret.
#[must_use = "check the Result for FIDO2 credential creation errors"]
pub fn make_credential_with_hmac(
    pin: Option<&str>,
    existing_credential_ids: &[Vec<u8>],
) -> Result<CredentialEnrollmentResult, Fido2Error> {
    let pin_owned = pin.map(|pin| Zeroizing::new(pin.to_string()));
    let existing_credential_ids_owned = existing_credential_ids.to_vec();

    with_fido_timeout("make_credential", move || {
        let pin = pin_owned.as_ref().map(|pin| pin.as_str());
        let device = open_device(&select_registration_device(
            &existing_credential_ids_owned,
            pin,
        )?)?;
        let credential = make_credential_with_device(&device, pin)?;
        let hmac_secret = get_hmac_secret_with_device(&device, &credential.credential_id, pin)?;
        Ok(CredentialEnrollmentResult {
            credential,
            hmac_secret,
        })
    })
}

/// Get the hmac-secret output for a given credential.
/// The output is deterministic for the same credential + application salt.
#[must_use = "check the Result for FIDO2 hmac-secret errors"]
pub fn get_hmac_secret(credential_id: &[u8], pin: Option<&str>) -> Result<[u8; 32], Fido2Error> {
    let cred_id_owned = credential_id.to_vec();
    let pin_owned = pin.map(|pin| Zeroizing::new(pin.to_string()));

    with_fido_timeout("get_hmac_secret", move || {
        let pin = pin_owned.as_ref().map(|pin| pin.as_str());
        let devices = ctap_hid_fido2::get_fidokey_devices();
        if devices.is_empty() {
            return Err(Fido2Error::NoDevice);
        }

        let mut last_error = None;
        for device_info in devices {
            let device = match open_device(&device_info.param) {
                Ok(device) => device,
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };

            match get_hmac_secret_with_device(&device, &cred_id_owned, pin) {
                Ok(hmac) => return Ok(hmac),
                Err(Fido2Error::NoMatchingCredential) => {}
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or(Fido2Error::NoMatchingCredential))
    })
}

#[cfg(test)]
mod tests {
    use super::{FIDO_DEVICE_LOCK, FIDO_DEVICE_LOCK_POISONED, classify_ctap_error};
    use crate::error::Fido2Error;

    const POISON_CHILD_ENV: &str = "SIGILLUM_TEST_POISONED_FIDO_DEVICE_LOCK_CHILD";

    #[test]
    fn poisoned_hid_lock_child_returns_restart_error() {
        if std::env::var_os(POISON_CHILD_ENV).is_none() {
            return;
        }
        let _ = std::thread::spawn(|| {
            let _guard = FIDO_DEVICE_LOCK.lock().unwrap();
            panic!("intentional FIDO device-lock poison");
        })
        .join();

        let error = super::with_fido_timeout("poison-test", || Ok(()))
            .expect_err("poisoned HID state must fail closed");
        assert!(
            matches!(error, Fido2Error::Other(message) if message == FIDO_DEVICE_LOCK_POISONED)
        );
    }

    #[test]
    fn poisoned_hid_lock_requires_restart_instead_of_recovery() {
        let status =
            std::process::Command::new(std::env::current_exe().expect("current test binary"))
                .args([
                    "--exact",
                    "hid::tests::poisoned_hid_lock_child_returns_restart_error",
                    "--nocapture",
                ])
                .env(POISON_CHILD_ENV, "1")
                .status()
                .expect("poison child should launch");
        assert!(status.success(), "poisoned HID child should fail closed");
    }

    #[test]
    fn classifies_pin_auth_blocked_errors() {
        let error = classify_ctap_error(
            "make_credential",
            "response_status err = 0x34 CTAP2_ERR_PIN_AUTH_BLOCKED Requires power recycle to reset",
        );
        assert!(matches!(error, Fido2Error::PinAuthBlocked));
    }

    #[test]
    fn classifies_pin_not_set_errors() {
        let error = classify_ctap_error(
            "make_credential",
            "response_status err = 0x35 CTAP2_ERR_PIN_NOT_SET No PIN has been set.",
        );
        assert!(matches!(error, Fido2Error::PinNotSet));
    }

    #[test]
    fn classifies_pin_required_errors() {
        let error = classify_ctap_error(
            "get_assertion",
            "response_status err = 0x36 CTAP2_ERR_PIN_REQUIRED PIN is required for the selected operation.",
        );
        assert!(matches!(error, Fido2Error::PinRequired));
    }

    #[test]
    fn classifies_no_matching_credential_errors() {
        let error = classify_ctap_error(
            "get_assertion",
            "response_status err = 0x2E CTAP2_ERR_NO_CREDENTIALS No valid credentials provided.",
        );
        assert!(matches!(error, Fido2Error::NoMatchingCredential));
    }

    #[test]
    fn classifies_pin_blocked_errors() {
        let error = classify_ctap_error(
            "get_assertion",
            "response_status err = 0x32 CTAP2_ERR_PIN_BLOCKED pin blocked",
        );
        assert!(matches!(error, Fido2Error::PinBlocked));
    }

    #[test]
    fn classifies_incorrect_pin_errors() {
        let error = classify_ctap_error(
            "get_assertion",
            "response_status err = 0x31 CTAP2_ERR_PIN_INVALID invalid pin",
        );
        assert!(matches!(error, Fido2Error::IncorrectPin));
    }

    #[test]
    fn short_new_pin_is_rejected_before_hid_access() {
        let error = super::set_new_pin("123").expect_err("short pin should be rejected");
        assert!(
            matches!(error, Fido2Error::Other(message) if message.contains("at least 4 characters"))
        );
    }
}
