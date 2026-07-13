use crate::{Fido2SetupResponse, UnlockResponse};

use super::super::is_canonical_session_token;

pub(super) fn unlock(response: &UnlockResponse, expected_method: &str) -> Result<(), String> {
    if response.status != "unlocked" || response.method != expected_method {
        return Err(format!(
            "unlock returned status {:?} and method {:?}, expected method {expected_method:?}",
            response.status, response.method
        ));
    }
    if !is_canonical_session_token(&response.session_token) {
        return Err("unlock returned a non-canonical session token".into());
    }
    let Some(active_id) = response.active_compartment_id else {
        return Err("unlock returned no active compartment".into());
    };
    if response.unlocked_compartments.is_empty()
        || !response
            .unlocked_compartments
            .iter()
            .any(|compartment| compartment.id == active_id)
    {
        return Err(format!(
            "unlock active compartment {active_id} was absent from unlocked_compartments"
        ));
    }
    Ok(())
}

pub(super) fn fido2_setup(
    response: &Fido2SetupResponse,
    expected_compartments: usize,
) -> Result<(), String> {
    if response.status != "setup_complete" || !response.unlocked {
        return Err(format!(
            "FIDO2 setup returned status {:?} with unlocked={}",
            response.status, response.unlocked
        ));
    }
    if !is_canonical_session_token(&response.session_token) {
        return Err("FIDO2 setup returned a non-canonical session token".into());
    }
    if response.total_keys == 0
        || response.compartments == 0
        || response.compartments != expected_compartments
    {
        return Err(format!(
            "FIDO2 setup returned total_keys={} and compartments={}, expected {} compartments",
            response.total_keys, response.compartments, expected_compartments
        ));
    }
    Ok(())
}
