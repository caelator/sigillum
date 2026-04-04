use std::io;

use libc::{c_void, size_t};
use zeroize::{Zeroize, Zeroizing};

pub(crate) struct PinnedSecretBytes {
    bytes: Zeroizing<Vec<u8>>,
}

impl PinnedSecretBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> io::Result<Self> {
        let mut secret = Self {
            bytes: Zeroizing::new(bytes),
        };
        secret.pin()?;
        Ok(secret)
    }

    pub(crate) fn as_array_32(&self) -> Option<[u8; 32]> {
        if self.bytes.len() != 32 {
            return None;
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.bytes);
        Some(out)
    }

    pub(crate) fn zeroize(&mut self) {
        self.bytes.zeroize();
    }

    fn pin(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        unsafe {
            let ptr = self.bytes.as_mut_ptr().cast::<c_void>();
            let len = self.bytes.len() as size_t;
            if libc::mlock(ptr, len) != 0 {
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
            let ptr = self.bytes.as_mut_ptr().cast::<c_void>();
            let len = self.bytes.len() as size_t;
            libc::munlock(ptr, len);
        }
        self.bytes.zeroize();
    }
}
