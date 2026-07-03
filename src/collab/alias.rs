// bole-0ms
use async_trait::async_trait;

use crate::collab::Key;
use crate::error::Result;

// bole-0ms
/// How a DNS/email-style alias relates to a key. An alias is NEVER authoritative
/// and NEVER a resolution key; this status only changes how the alias is
/// *displayed*. Keys remain canonical regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasStatus {
    /// The claimed domain asserts exactly this key.
    Verified,
    /// The alias is claimed but the domain does not (or cannot) assert this key.
    Claimed,
}

/// Resolves what key, if any, a domain asserts for an alias. The production impl
/// fetches `https://<domain>/.well-known/bole-key` (or a TXT record); tests inject
/// a mock. Errors here are surfaced but must never be treated as identity loss.
#[async_trait]
pub trait AliasResolver {
    async fn asserted_key(&self, alias: &str) -> Result<Option<Key>>;
}

/// `Verified` iff the resolver reports the domain asserts exactly `key`;
/// otherwise `Claimed`. Never returns a key and never promotes an alias to
/// authority.
pub async fn verify_alias(resolver: &impl AliasResolver, alias: &str, key: &Key) -> Result<AliasStatus> {
    match resolver.asserted_key(alias).await? {
        Some(asserted) if &asserted == key => Ok(AliasStatus::Verified),
        _ => Ok(AliasStatus::Claimed),
    }
}

// bole-0ms
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Test resolver: an in-memory map from alias -> the key the "domain" asserts,
    /// standing in for a `.well-known/bole-key` fetch or TXT lookup.
    struct MockDns(BTreeMap<String, Key>);

    #[async_trait]
    impl AliasResolver for MockDns {
        async fn asserted_key(&self, alias: &str) -> Result<Option<Key>> {
            Ok(self.0.get(alias).copied())
        }
    }

    #[tokio::test]
    async fn alias_verified_when_domain_asserts_key() {
        let alice = [1u8; 32];
        let mut m = BTreeMap::new();
        m.insert("alice@bole.dev".to_string(), alice);
        let dns = MockDns(m);
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &alice).await.unwrap(), AliasStatus::Verified);
    }

    #[tokio::test]
    async fn conflicting_alias_stays_claimed_key_canonical() {
        let alice = [1u8; 32];
        let mallory = [2u8; 32];
        // The domain asserts alice's key, but mallory ALSO claims the alias.
        let mut m = BTreeMap::new();
        m.insert("alice@bole.dev".to_string(), alice);
        let dns = MockDns(m);

        // Mallory's claim does not verify: the domain does not assert mallory's key.
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &mallory).await.unwrap(), AliasStatus::Claimed);
        // And the canonical identity is unchanged — verify_alias never returns a key.
        assert_eq!(verify_alias(&dns, "alice@bole.dev", &alice).await.unwrap(), AliasStatus::Verified);

        // Unknown domain / no assertion -> Claimed, never an error that blocks use.
        let empty = MockDns(BTreeMap::new());
        assert_eq!(verify_alias(&empty, "ghost@nowhere.example", &alice).await.unwrap(), AliasStatus::Claimed);
    }
}
