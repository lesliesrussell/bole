// bole-3xj5
//! Auth configuration loaded from TOML. Extended in later tasks; for now it is
//! an empty-by-default holder so `AppState` can carry it.

use std::collections::HashMap;

use serde::Deserialize;

/// A registered signing key: its ed25519 public key (32 bytes) and the actor it
/// authenticates as.
#[derive(Debug, Clone)]
pub struct RegisteredKey {
    pub pubkey: [u8; 32],
    pub actor: String,
}

/// Parsed auth configuration.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub actors: bole::sync::authn::ActorMap,
    pub keys: HashMap<String, RegisteredKey>,
    pub trusted_proxies: Vec<String>,
}

/// The on-disk TOML shape.
#[derive(Debug, Default, Deserialize)]
pub struct AuthConfigFile {
    #[serde(default)]
    pub tokens: HashMap<String, String>,
    #[serde(default)]
    pub mtls: HashMap<String, String>,
    #[serde(default)]
    pub keys: HashMap<String, KeyEntry>,
    #[serde(default)]
    pub proxy: ProxySection,
}

#[derive(Debug, Deserialize)]
pub struct KeyEntry {
    pub pubkey: String,
    pub actor: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProxySection {
    #[serde(default)]
    pub trusted: Vec<String>,
}

impl AuthConfig {
    /// Builds runtime config from the parsed file, validating hex fields.
    pub fn from_file(f: AuthConfigFile) -> anyhow::Result<Self> {
        let mut actors = bole::sync::authn::ActorMap::new();
        for (token, actor) in f.tokens {
            actors.map_token(token, actor);
        }
        for (subject, actor) in f.mtls {
            actors.map_mtls(subject, actor);
        }
        let mut keys = HashMap::new();
        for (key_id, entry) in f.keys {
            let raw = hex::decode(&entry.pubkey)
                .map_err(|_| anyhow::anyhow!("key {key_id}: pubkey is not valid hex"))?;
            let pubkey: [u8; 32] = raw
                .try_into()
                .map_err(|_| anyhow::anyhow!("key {key_id}: pubkey must be 32 bytes"))?;
            actors.map_ssh_key(key_id.clone(), entry.actor.clone());
            keys.insert(key_id, RegisteredKey { pubkey, actor: entry.actor });
        }
        Ok(Self { actors, keys, trusted_proxies: f.proxy.trusted })
    }

    /// Parses a TOML string into runtime config.
    pub fn parse(toml_str: &str) -> anyhow::Result<Self> {
        let file: AuthConfigFile = toml::from_str(toml_str)?;
        Self::from_file(file)
    }
}
