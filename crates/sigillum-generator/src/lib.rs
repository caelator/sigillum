//! Cryptographic generators for passwords, passphrases, and TOTPs.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE32_NOPAD;
use rand::RngCore;
use rand::rngs::OsRng;

const LOWER_ALPHA: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER_ALPHA: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const NUMERIC: &str = "0123456789";
const SYMBOL: &str = "!@#$%^&*()-_=+[]{}<>?,.";
const WORDS: &[&str] = &[
    "anchor", "apricot", "atlas", "badger", "bamboo", "banner", "beacon", "birch", "bistro",
    "brisk", "cactus", "canvas", "cinder", "cobalt", "comet", "copper", "coral", "cradle", "dawn",
    "delta", "ember", "falcon", "fable", "fjord", "forest", "frost", "garnet", "glimmer", "harbor",
    "hazel", "helium", "indigo", "iris", "jade", "jasmine", "jovial", "kernel", "lagoon",
    "lantern", "lilac", "linen", "lotus", "marble", "meadow", "meteor", "mint", "mosaic", "nebula",
    "nickel", "north", "novel", "onyx", "opal", "orchid", "otter", "pebble", "pepper", "petal",
    "photon", "pine", "plume", "poppy", "quartz", "quill", "radar", "raven", "reef", "river",
    "sable", "saffron", "sage", "scarlet", "shadow", "signal", "silver", "spruce", "starling",
    "stone", "summit", "teal", "tempo", "thistle", "timber", "topaz", "torrent", "trident",
    "tulip", "umbra", "valley", "velvet", "violet", "walnut", "warden", "willow", "winter",
    "yarrow", "zephyr",
];

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
    let mut words = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        let index = (rng.next_u64() as usize) % WORDS.len();
        words.push(WORDS[index]);
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
    let hmac = hmac_sha1(&key, &message);
    let offset = (hmac[19] & 0x0f) as usize;
    let binary = ((u32::from(hmac[offset]) & 0x7f) << 24)
        | (u32::from(hmac[offset + 1]) << 16)
        | (u32::from(hmac[offset + 2]) << 8)
        | u32::from(hmac[offset + 3]);
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

fn sample_uniform(rng: &mut OsRng, upper_bound: u64) -> u64 {
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

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..20].copy_from_slice(&sha1(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= block[i];
        opad[i] ^= block[i];
    }

    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(message);
    let inner_hash = sha1(&inner);

    let mut outer = Vec::with_capacity(64 + inner_hash.len());
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha1(&outer)
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xefcd_ab89;
    let mut h2: u32 = 0x98ba_dcfe;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xc3d2_e1f0;

    let bit_len = (input.len() as u64) * 8;
    let mut data = input.to_vec();
    data.push(0x80);
    while (data.len() % 64) != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let start = i * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => (((b & c) | ((!b) & d)), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => (((b & c) | (b & d) | (c & d)), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn totp_matches_rfc_vector() {
        let code = generate_totp_at("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59, 30, 8).unwrap();
        assert_eq!(code, "94287082");
    }
}
