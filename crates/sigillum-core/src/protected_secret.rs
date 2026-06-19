use std::io;

use zeroize::{Zeroize, Zeroizing};

pub struct PinnedSecretBytes {
    bytes: Zeroizing<Vec<u8>>,
    pinned: bool,
}

impl PinnedSecretBytes {
    pub fn new(bytes: Vec<u8>) -> io::Result<Self> {
        let mut secret = Self {
            bytes: Zeroizing::new(bytes),
            pinned: false,
        };
        secret.pin()?;
        secret.pinned = true;
        Ok(secret)
    }

    pub fn new_lossy(bytes: Vec<u8>) -> Self {
        let mut secret = Self {
            bytes: Zeroizing::new(bytes),
            pinned: false,
        };
        if secret.pin().is_ok() {
            secret.pinned = true;
        }
        secret
    }

    pub fn from_array_32_lossy(bytes: [u8; 32]) -> Self {
        Self::new_lossy(bytes.to_vec())
    }

    pub fn as_array_32(&self) -> Option<[u8; 32]> {
        if self.bytes.len() != 32 {
            return None;
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.bytes);
        Some(out)
    }

    pub fn with_array_32<T>(&self, f: impl FnOnce(&[u8; 32]) -> T) -> Option<T> {
        let key: &[u8; 32] = self.bytes.as_slice().try_into().ok()?;
        Some(f(key))
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn zeroize(&mut self) {
        self.bytes.zeroize();
    }

    fn pin(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        unsafe {
            let ptr = self.bytes.as_mut_ptr().cast::<libc::c_void>();
            let len = self.bytes.len() as libc::size_t;
            if len > 0 && libc::mlock(ptr, len) != 0 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }
}

impl Drop for PinnedSecretBytes {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            if self.pinned && !self.bytes.is_empty() {
                let ptr = self.bytes.as_mut_ptr().cast::<libc::c_void>();
                let len = self.bytes.len() as libc::size_t;
                libc::munlock(ptr, len);
            }
        }
        self.bytes.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::PinnedSecretBytes;

    #[test]
    fn pinned_secret_roundtrips_array_32() {
        let secret = PinnedSecretBytes::from_array_32_lossy([7u8; 32]);
        assert_eq!(secret.as_array_32(), Some([7u8; 32]));
        let copied = secret.with_array_32(|bytes| *bytes).unwrap();
        assert_eq!(copied, [7u8; 32]);
    }
}
