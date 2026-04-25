//! Cryptographic generators for passwords, passphrases, and TOTPs.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use bip39::Language;
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rand::rngs::OsRng;
use sha1::Sha1;

const LOWER_ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER_ALPHA: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NUMERIC: &str = "0123456789";
const SYMBOL: &str = "!@#$%^&*()-_=+[]{}<>?,.";

pub const DEFAULT_PASSPHRASE_WORDS: usize = 8;

type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratorError {
    InvalidLength(&'static str),
    UnsupportedCharset(String),
    InvalidSecret(&'static str),
    InvalidDigits,
    TimeUnavailable,
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength(message) => write!(f, "{message}"),
            Self::UnsupportedCharset(charset) => {
                write!(f, "unsupported charset '{charset}'")
            }
            Self::InvalidSecret(message) => write!(f, "{message}"),
            Self::InvalidDigits => write!(f, "digits must be between 6 and 8"),
            Self::TimeUnavailable => write!(f, "system clock is before the Unix epoch"),
        }
    }
}

impl std::error::Error for GeneratorError {}

pub fn generate_password(charset: &str, length: usize) -> Result<String, GeneratorError> {
    if length == 0 {
        return Err(GeneratorError::InvalidLength(
            "password length must be greater than zero",
        ));
    }

    let alphabet = match charset {
        "loweralpha" => LOWER_ALPHA.to_string(),
        "mixalpha" => concat_const(LOWER_ALPHA, UPPER_ALPHA),
        "numeric" => NUMERIC.to_string(),
        "alpha-numeric" => concat_const(LOWER_ALPHA, UPPER_ALPHA_NUMERIC),
        "mixalpha-numeric" => concat_const(LOWER_ALPHA, UPPER_ALPHA_NUMERIC),
        "mixalpha-numeric-symbol" => concat_const3(LOWER_ALPHA, UPPER_ALPHA_NUMERIC, SYMBOL),
        other => return Err(GeneratorError::UnsupportedCharset(other.to_string())),
    };

    sample_chars(alphabet.as_bytes(), length)
}

pub fn generate_passphrase(word_count: usize, separator: &str) -> Result<String, GeneratorError> {
    if word_count == 0 {
        return Err(GeneratorError::InvalidLength(
            "word count must be greater than zero",
        ));
    }

    let mut rng = OsRng;
    let word_list = Language::English.word_list();
    let mut words = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        let index = sample_uniform(&mut rng, word_list.len() as u64) as usize;
        words.push(word_list[index]);
    }
    Ok(words.join(separator))
}

pub fn generate_totp(secret: &str, period: u64, digits: u32) -> Result<String, GeneratorError> {
    if period == 0 {
        return Err(GeneratorError::InvalidLength(
            "period must be greater than zero",
        ));
    }
    if !(6..=8).contains(&digits) {
        return Err(GeneratorError::InvalidDigits);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GeneratorError::TimeUnavailable)?
        .as_secs();
    let remaining = period - (now % period);
    let effective_now = if remaining <= 1 {
        now.saturating_add(remaining)
    } else {
        now
    };
    generate_totp_at(secret, effective_now, period, digits)
}

fn generate_totp_at(
    secret: &str,
    timestamp: u64,
    period: u64,
    digits: u32,
) -> Result<String, GeneratorError> {
    let normalized = secret
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect::<String>()
        .to_uppercase();
    let key = BASE32_NOPAD
        .decode(normalized.as_bytes())
        .map_err(|_| GeneratorError::InvalidSecret("secret must be valid base32"))?;
    if key.is_empty() {
        return Err(GeneratorError::InvalidSecret(
            "secret must decode to at least one byte",
        ));
    }

    let counter = timestamp / period;
    let mut message = [0u8; 8];
    message.copy_from_slice(&counter.to_be_bytes());
    let mut mac = HmacSha1::new_from_slice(&key)
        .map_err(|_| GeneratorError::InvalidSecret("failed to initialize TOTP HMAC"))?;
    mac.update(&message);
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let modulo = 10_u32.pow(digits);
    let code = binary % modulo;
    Ok(format!("{code:0width$}", width = digits as usize))
}

fn sample_chars(alphabet: &[u8], length: usize) -> Result<String, GeneratorError> {
    let mut output = String::with_capacity(length);
    let mut rng = OsRng;
    for _ in 0..length {
        let idx = sample_uniform(&mut rng, alphabet.len() as u64) as usize;
        output.push(alphabet[idx] as char);
    }
    Ok(output)
}

fn sample_uniform<R: RngCore + ?Sized>(rng: &mut R, upper_bound: u64) -> u64 {
    debug_assert!(upper_bound > 0);
    let zone = u64::MAX - (u64::MAX % upper_bound);
    loop {
        let value = rng.next_u64();
        if value < zone {
            return value % upper_bound;
        }
    }
}

const UPPER_ALPHA_NUMERIC: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn concat_const(a: &str, b: &str) -> String {
    let mut s = String::with_capacity(a.len() + b.len());
    s.push_str(a);
    s.push_str(b);
    s
}

fn concat_const3(a: &str, b: &str, c: &str) -> String {
    let mut s = String::with_capacity(a.len() + b.len() + c.len());
    s.push_str(a);
    s.push_str(b);
    s.push_str(c);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Error;

    struct FixedRng {
        values: Vec<u64>,
    }

    impl RngCore for FixedRng {
        fn next_u32(&mut self) -> u32 {
            self.next_u64() as u32
        }

        fn next_u64(&mut self) -> u64 {
            self.values.remove(0)
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            for chunk in dest.chunks_mut(8) {
                let value = self.next_u64().to_be_bytes();
                let len = chunk.len();
                chunk.copy_from_slice(&value[..len]);
            }
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    #[test]
    fn password_uses_requested_length() {
        let generated = generate_password("mixalpha-numeric-symbol", 32).unwrap();
        assert_eq!(generated.len(), 32);
    }

    #[test]
    fn passphrase_uses_requested_word_count() {
        let generated = generate_passphrase(5, "-").unwrap();
        assert_eq!(generated.split('-').count(), 5);
    }

    #[test]
    fn passphrase_uses_bundled_2048_word_list() {
        let word_list = Language::English.word_list();
        assert_eq!(word_list.len(), 2048);

        let generated = generate_passphrase(12, " ").unwrap();
        assert!(generated.split(' ').all(|word| word_list.contains(&word)));
    }

    #[test]
    fn sample_uniform_rejects_out_of_zone_values() {
        let mut rng = FixedRng {
            values: vec![u64::MAX, 7],
        };
        assert_eq!(sample_uniform(&mut rng, 10), 7);
    }

    #[test]
    fn totp_matches_rfc_vector() {
        let code = generate_totp_at("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 8).unwrap();
        assert_eq!(code, "94287082");
    }

    #[test]
    fn totp_rejects_invalid_inputs() {
        assert_eq!(
            generate_totp("not-base32", 30, 6).unwrap_err(),
            GeneratorError::InvalidSecret("secret must be valid base32")
        );
        assert_eq!(
            generate_totp("GEZDGNBVGY3TQOJQ", 0, 6).unwrap_err(),
            GeneratorError::InvalidLength("period must be greater than zero")
        );
        assert_eq!(
            generate_totp("GEZDGNBVGY3TQOJQ", 30, 9).unwrap_err(),
            GeneratorError::InvalidDigits
        );
    }
}
