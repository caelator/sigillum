use std::io;

use hkdf::Hkdf;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;
use sigillum_core::VaultLifecycle;

use super::AppState;

impl AppState {
    pub(super) fn audit_chain_scope_and_key(
        &self,
        compartment_id: Option<usize>,
    ) -> Result<(String, [u8; 32]), std::io::Error> {
        match compartment_id {
            Some(id) => {
                let scope = format!("compartment:{id}");
                let key = self.compartment_audit_key(id)?;
                Ok((scope, key))
            }
            None => Ok(("daemon".into(), self.daemon_audit_key()?)),
        }
    }

    pub(super) fn audit_key_for_scope(&self, scope: &str) -> Result<[u8; 32], std::io::Error> {
        if scope == "daemon" {
            return self.daemon_audit_key();
        }
        let id = scope
            .strip_prefix("compartment:")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "audit scope must be daemon or compartment:<id>",
                )
            })?;
        self.compartment_audit_key(id)
    }

    fn daemon_audit_key(&self) -> Result<[u8; 32], std::io::Error> {
        let path = self.audit_key_path();
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Ok(key)
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "audit.key must contain 32 bytes",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut key = [0u8; 32];
                OsRng.fill_bytes(&mut key);
                std::fs::write(&path, key)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                }
                Ok(key)
            }
            Err(error) => Err(error),
        }
    }

    fn compartment_audit_key(&self, id: usize) -> Result<[u8; 32], std::io::Error> {
        let master_key = {
            let vaults = self.vaults.lock();
            vaults
                .get(&id)
                .and_then(|vault| vault.extract_master_key())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "compartment audit verification requires an unlocked compartment",
                    )
                })?
        };
        let hkdf = Hkdf::<Sha256>::new(None, master_key.as_ref());
        let mut key = [0u8; 32];
        hkdf.expand(
            format!("sigillum/audit-hmac/v1/compartment:{id}").as_bytes(),
            &mut key,
        )
        .map_err(|_| io::Error::other("failed to derive audit HMAC key"))?;
        Ok(key)
    }
}
