//! Secret-generation request contracts.

use serde::{Deserialize, Serialize};

/// Supported CLI password generator character sets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PasswordCharset {
    Loweralpha,
    Mixalpha,
    Numeric,
    AlphaNumeric,
    MixalphaNumeric,
    MixalphaNumericSymbol,
}

impl PasswordCharset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Loweralpha => "loweralpha",
            Self::Mixalpha => "mixalpha",
            Self::Numeric => "numeric",
            Self::AlphaNumeric => "alpha-numeric",
            Self::MixalphaNumeric => "mixalpha-numeric",
            Self::MixalphaNumericSymbol => "mixalpha-numeric-symbol",
        }
    }
}

/// Atomically generate a secret value and persist it in the active compartment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GenerateStoreRequest {
    pub key: String,
    pub kind: GenerateStoreKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum GenerateStoreKind {
    Password {
        length: usize,
        charset: PasswordCharset,
    },
    Passphrase {
        word_count: usize,
        separator: String,
    },
}
