use secrecy::SecretString;
use zeroize::Zeroizing;

use crate::VaultError;

pub trait UnlockProvider: Send + Sync {
    fn method_name(&self) -> &'static str;
    fn unlock_master_key(&self) -> Result<Zeroizing<[u8; 32]>, VaultError>;
}

pub struct PassphraseUnlockProvider<F>
where
    F: Fn(&SecretString) -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    passphrase: SecretString,
    resolver: F,
}

impl<F> PassphraseUnlockProvider<F>
where
    F: Fn(&SecretString) -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    pub fn new(passphrase: SecretString, resolver: F) -> Self {
        Self {
            passphrase,
            resolver,
        }
    }
}

impl<F> UnlockProvider for PassphraseUnlockProvider<F>
where
    F: Fn(&SecretString) -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    fn method_name(&self) -> &'static str {
        "passphrase"
    }

    fn unlock_master_key(&self) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        (self.resolver)(&self.passphrase)
    }
}

pub struct Fido2UnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    resolver: F,
}

impl<F> Fido2UnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    pub fn new(resolver: F) -> Self {
        Self { resolver }
    }
}

impl<F> UnlockProvider for Fido2UnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    fn method_name(&self) -> &'static str {
        "fido2"
    }

    fn unlock_master_key(&self) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        (self.resolver)()
    }
}

pub struct TouchIdUnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    resolver: F,
}

impl<F> TouchIdUnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    pub fn new(resolver: F) -> Self {
        Self { resolver }
    }
}

impl<F> UnlockProvider for TouchIdUnlockProvider<F>
where
    F: Fn() -> Result<Zeroizing<[u8; 32]>, VaultError> + Send + Sync,
{
    fn method_name(&self) -> &'static str {
        "touch-id"
    }

    fn unlock_master_key(&self) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        (self.resolver)()
    }
}
