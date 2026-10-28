use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How a tree names an asset in an image `src`: this, then the asset id.
pub const ASSET_SRC_PREFIX: &str = "asset:";

const HEX_LEN: usize = 64;

/// The lowercase hex SHA-256 of an asset's bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AssetId(String);

impl AssetId {
    pub fn parse(text: &str) -> Option<Self> {
        let well_formed = text.len() == HEX_LEN
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        well_formed.then(|| Self(text.to_owned()))
    }

    pub fn of(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        Self(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// The asset named by an image `src`, if it names one.
    pub fn from_src(src: &str) -> Option<Self> {
        src.strip_prefix(ASSET_SRC_PREFIX).and_then(Self::parse)
    }

    pub fn to_src(&self) -> String {
        format!("{ASSET_SRC_PREFIX}{}", self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, bytes: &[u8]) -> bool {
        Self::of(bytes) == *self
    }
}

impl TryFrom<String> for AssetId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or_else(|| format!("`{text}` is not a SHA-256 asset id"))
    }
}

impl From<AssetId> for String {
    fn from(id: AssetId) -> Self {
        id.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_the_hash_of_their_bytes() {
        let id = AssetId::of(b"abc");
        assert_eq!(
            id.as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(id.matches(b"abc"));
        assert!(!id.matches(b"abd"));
        assert_eq!(AssetId::from_src(&id.to_src()), Some(id));
    }

    #[test]
    fn only_lowercase_sha256_hex_parses() {
        assert!(AssetId::parse("../../etc/passwd").is_none());
        assert!(AssetId::parse(&"A".repeat(64)).is_none());
        assert!(AssetId::parse(&"a".repeat(63)).is_none());
        assert!(AssetId::parse(&"a".repeat(64)).is_some());
    }
}
