use zeroize::{Zeroize, Zeroizing};

const CURRENT_VERSION: u8 = 1;
const CHALLENGE_ID_LEN: usize = 16;
const MAX_PROOF_LEN: usize = 128;
const MAX_KEY_LEN: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiometricUnlockPayload {
    pub version: u8,
    pub challenge_id: [u8; CHALLENGE_ID_LEN],
    pub proof: Vec<u8>,
    pub key_encoding: u8,
    pub key: Zeroizing<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiometricHelperOutput {
    pub version: u8,
    pub proof: Vec<u8>,
    pub key_encoding: u8,
    pub key: Zeroizing<Vec<u8>>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum BiometricPayloadError {
    #[error("payload is truncated")]
    Truncated,
    #[error("unsupported payload version {0}")]
    UnsupportedVersion(u8),
    #[error("proof length {0} exceeds maximum {MAX_PROOF_LEN}")]
    ProofTooLong(usize),
    #[error("key length {0} exceeds maximum {MAX_KEY_LEN}")]
    KeyTooLong(usize),
    #[error("unexpected trailing bytes")]
    TrailingBytes,
}

impl BiometricUnlockPayload {
    pub fn new(
        challenge_id: [u8; CHALLENGE_ID_LEN],
        proof: Vec<u8>,
        key_encoding: u8,
        key: Vec<u8>,
    ) -> Result<Self, BiometricPayloadError> {
        if proof.len() > MAX_PROOF_LEN {
            return Err(BiometricPayloadError::ProofTooLong(proof.len()));
        }
        if key.len() > MAX_KEY_LEN {
            return Err(BiometricPayloadError::KeyTooLong(key.len()));
        }
        Ok(Self {
            version: CURRENT_VERSION,
            challenge_id,
            proof,
            key_encoding,
            key: Zeroizing::new(key),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(1 + CHALLENGE_ID_LEN + 2 + self.proof.len() + 1 + 2 + self.key.len());
        out.push(self.version);
        out.extend_from_slice(&self.challenge_id);
        out.extend_from_slice(&(self.proof.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.proof);
        out.push(self.key_encoding);
        out.extend_from_slice(&(self.key.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.key);
        out
    }

    pub fn decode(input: &[u8]) -> Result<Self, BiometricPayloadError> {
        let mut cursor = Cursor::new(input);
        let version = cursor.read_u8()?;
        if version != CURRENT_VERSION {
            return Err(BiometricPayloadError::UnsupportedVersion(version));
        }

        let challenge_id = cursor.read_fixed()?;
        let proof_len = usize::from(cursor.read_u16()?);
        if proof_len > MAX_PROOF_LEN {
            return Err(BiometricPayloadError::ProofTooLong(proof_len));
        }
        let proof = cursor.read_vec(proof_len)?;

        let key_encoding = cursor.read_u8()?;
        let key_len = usize::from(cursor.read_u16()?);
        if key_len > MAX_KEY_LEN {
            return Err(BiometricPayloadError::KeyTooLong(key_len));
        }
        let key = Zeroizing::new(cursor.read_vec(key_len)?);

        if !cursor.is_eof() {
            return Err(BiometricPayloadError::TrailingBytes);
        }

        Ok(Self {
            version,
            challenge_id,
            proof,
            key_encoding,
            key,
        })
    }
}

impl BiometricHelperOutput {
    pub fn new(proof: Vec<u8>, key_encoding: u8, key: Vec<u8>) -> Result<Self, BiometricPayloadError> {
        if proof.len() > MAX_PROOF_LEN {
            return Err(BiometricPayloadError::ProofTooLong(proof.len()));
        }
        if key.len() > MAX_KEY_LEN {
            return Err(BiometricPayloadError::KeyTooLong(key.len()));
        }
        Ok(Self {
            version: CURRENT_VERSION,
            proof,
            key_encoding,
            key: Zeroizing::new(key),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 2 + self.proof.len() + 1 + 2 + self.key.len());
        out.push(self.version);
        out.extend_from_slice(&(self.proof.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.proof);
        out.push(self.key_encoding);
        out.extend_from_slice(&(self.key.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.key);
        out
    }

    pub fn decode(input: &[u8]) -> Result<Self, BiometricPayloadError> {
        let mut cursor = Cursor::new(input);
        let version = cursor.read_u8()?;
        if version != CURRENT_VERSION {
            return Err(BiometricPayloadError::UnsupportedVersion(version));
        }

        let proof_len = usize::from(cursor.read_u16()?);
        if proof_len > MAX_PROOF_LEN {
            return Err(BiometricPayloadError::ProofTooLong(proof_len));
        }
        let proof = cursor.read_vec(proof_len)?;

        let key_encoding = cursor.read_u8()?;
        let key_len = usize::from(cursor.read_u16()?);
        if key_len > MAX_KEY_LEN {
            return Err(BiometricPayloadError::KeyTooLong(key_len));
        }
        let key = Zeroizing::new(cursor.read_vec(key_len)?);

        if !cursor.is_eof() {
            return Err(BiometricPayloadError::TrailingBytes);
        }

        Ok(Self {
            version,
            proof,
            key_encoding,
            key,
        })
    }

    pub fn into_payload(
        self,
        challenge_id: [u8; CHALLENGE_ID_LEN],
    ) -> Result<BiometricUnlockPayload, BiometricPayloadError> {
        BiometricUnlockPayload::new(challenge_id, self.proof, self.key_encoding, self.key.to_vec())
    }
}

struct Cursor<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, BiometricPayloadError> {
        if self.offset >= self.input.len() {
            return Err(BiometricPayloadError::Truncated);
        }
        let value = self.input[self.offset];
        self.offset += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, BiometricPayloadError> {
        let bytes = self.read_vec(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_fixed<const N: usize>(&mut self) -> Result<[u8; N], BiometricPayloadError> {
        let bytes = self.read_vec(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(&bytes);
        Ok(out)
    }

    fn read_vec(&mut self, len: usize) -> Result<Vec<u8>, BiometricPayloadError> {
        let end = self.offset.checked_add(len).ok_or(BiometricPayloadError::Truncated)?;
        if end > self.input.len() {
            return Err(BiometricPayloadError::Truncated);
        }
        let bytes = self.input[self.offset..end].to_vec();
        self.offset = end;
        Ok(bytes)
    }

    fn is_eof(&self) -> bool {
        self.offset == self.input.len()
    }
}

impl Drop for Cursor<'_> {
    fn drop(&mut self) {
        let mut remaining = self.input[self.offset..].to_vec();
        remaining.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::{BiometricHelperOutput, BiometricPayloadError, BiometricUnlockPayload};

    #[test]
    fn payload_round_trip() {
        let payload = BiometricUnlockPayload::new([7u8; 16], vec![1, 2, 3], 1, vec![9u8; 32]).unwrap();
        let decoded = BiometricUnlockPayload::decode(&payload.encode()).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.challenge_id, [7u8; 16]);
        assert_eq!(decoded.proof, vec![1, 2, 3]);
        assert_eq!(&*decoded.key, &vec![9u8; 32]);
    }

    #[test]
    fn helper_output_round_trip() {
        let output = BiometricHelperOutput::new(vec![4u8; 8], 1, vec![5u8; 32]).unwrap();
        let decoded = BiometricHelperOutput::decode(&output.encode()).unwrap();
        assert_eq!(decoded.proof, vec![4u8; 8]);
        assert_eq!(&*decoded.key, &vec![5u8; 32]);
    }

    #[test]
    fn proof_is_checked_before_key_decode() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&(129u16).to_be_bytes());
        bytes.extend(std::iter::repeat_n(0u8, 10));
        bytes.push(1);
        bytes.extend_from_slice(&(200u16).to_be_bytes());
        let err = BiometricUnlockPayload::decode(&bytes).unwrap_err();
        assert_eq!(err, BiometricPayloadError::ProofTooLong(129));
    }

    #[test]
    fn key_length_is_bounded() {
        let err = BiometricHelperOutput::new(vec![1u8; 2], 1, vec![7u8; 129]).unwrap_err();
        assert_eq!(err, BiometricPayloadError::KeyTooLong(129));
    }
}
