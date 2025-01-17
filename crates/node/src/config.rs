//! Contains the configuration for the hilo-node.

use crate::SyncMode;
use alloy_rpc_types_engine::JwtSecret;
use maili_genesis::RollupConfig;
use serde::{Deserialize, Serialize};
use url::Url;

/// An error thrown by a [Config] operation.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// An error thrown by the beacon client.
    #[error("beacon client error: {0}")]
    Beacon(String),
    /// An L2 chain provider error.
    #[error("L2 chain provider error: {0}")]
    L2ChainProvider(String),
    /// An L1 chain provider error.
    #[error("L1 chain provider error: {0}")]
    ChainProvider(String),
}

/// The global node configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// The L2 Chain ID.
    pub l2_chain_id: u64,
    /// The L1 chain RPC URL
    pub l1_rpc_url: Url,
    /// The base chain beacon client RPC URL
    pub l1_beacon_url: Url,
    /// An optional blob archiver URL used in the fallback provider.
    pub blob_archiver_url: Option<Url>,
    /// The L2 chain RPC URL
    pub l2_rpc_url: Url,
    /// The L2 engine API URL
    pub l2_engine_url: Url,
    /// The rollup config
    pub rollup_config: RollupConfig,
    /// Engine API JWT Secret.
    /// This is used to authenticate with the engine API
    #[serde(deserialize_with = "deserialize_jwt_secret", serialize_with = "as_hex")]
    pub jwt_secret: JwtSecret,
    /// A trusted L2 RPC URL to use for fast/checkpoint syncing
    pub checkpoint_sync_url: Option<Url>,
    /// The hilo-node RPC server
    pub rpc_url: Option<Url>,
    /// The devnet mode.
    /// If devnet is enabled.
    pub devnet: bool,
    /// The mode to sync.
    pub sync_mode: SyncMode,
    /// The cache size for in-memory providers.
    pub cache_size: usize,
}

impl From<Config> for hilo_driver::Config {
    fn from(config: Config) -> Self {
        hilo_driver::Config {
            l2_chain_id: config.l2_chain_id,
            l1_rpc_url: config.l1_rpc_url,
            l1_beacon_url: config.l1_beacon_url,
            blob_archiver_url: config.blob_archiver_url,
            l2_rpc_url: config.l2_rpc_url,
            l2_engine_url: config.l2_engine_url,
            rollup_config: config.rollup_config,
            rpc_url: config.rpc_url,
            cache_size: config.cache_size,
            jwt_secret: config.jwt_secret,
        }
    }
}

fn as_hex<S>(v: &JwtSecret, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    let encoded = alloy_primitives::hex::encode(v.as_bytes());
    serializer.serialize_str(&encoded)
}

fn deserialize_jwt_secret<'de, D>(deserializer: D) -> Result<JwtSecret, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let s: &str = serde::de::Deserialize::deserialize(deserializer)?;
    JwtSecret::from_hex(s).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maili_registry::ROLLUP_CONFIGS;

    #[test]
    fn test_roundtrip_config() {
        let rollup_config = ROLLUP_CONFIGS.get(&10).unwrap().clone();
        let l1_rpc_url = Url::parse("http://127.0.0.1:8545").unwrap();
        let l1_beacon_url = Url::parse("http://127.0.0.1:8555").unwrap();
        let l2_rpc_url = Url::parse("http://127.0.0.1:9545").unwrap();
        let l2_engine_url = Url::parse("http://127.0.0.1:9555").unwrap();
        let jwt_secret = JwtSecret::random();
        let config = Config {
            l2_chain_id: 10,
            l1_rpc_url,
            l2_rpc_url,
            l1_beacon_url,
            blob_archiver_url: None,
            l2_engine_url,
            rollup_config,
            jwt_secret,
            checkpoint_sync_url: None,
            rpc_url: None,
            devnet: false,
            sync_mode: SyncMode::Fast,
            cache_size: 256,
        };

        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }
}
