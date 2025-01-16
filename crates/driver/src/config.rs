//! Configuration for the Hilo Driver.

use alloy_rpc_types_engine::JwtSecret;
use kona_derive::traits::ChainProvider;
use kona_driver::PipelineCursor;
use maili_genesis::RollupConfig;
use maili_protocol::{BatchValidationProvider, BlockInfo, L2BlockInfo};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

use hilo_providers_alloy::{
    AlloyChainProvider, AlloyL2ChainProvider, BeaconClient, OnlineBeaconClient, OnlineBlobProvider,
    OnlineBlobProviderWithFallback,
};

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
    /// The hilo-node RPC server
    pub rpc_url: Option<Url>,
    /// Engine API JWT Secret.
    /// This is used to authenticate with the engine API
    #[serde(deserialize_with = "deserialize_jwt_secret", serialize_with = "as_hex")]
    pub jwt_secret: JwtSecret,
    /// The cache size for in-memory providers.
    pub cache_size: usize,
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

impl Config {
    /// Construct an [OnlineBlobProviderWithFallback] from the [Config].
    pub async fn blob_provider(
        &self,
    ) -> Result<OnlineBlobProviderWithFallback<OnlineBeaconClient, OnlineBeaconClient>, ConfigError>
    {
        let beacon_client = OnlineBeaconClient::new_http(String::from(self.l1_beacon_url.clone()));
        let genesis = Some(self.rollup_config.genesis.l1.number);
        let slot_interval = beacon_client
            .config_spec()
            .await
            .map_err(|e| ConfigError::Beacon(e.to_string()))?
            .data
            .seconds_per_slot;
        let blob = OnlineBlobProvider::new(beacon_client, genesis, Some(slot_interval));
        Ok(OnlineBlobProviderWithFallback::new(blob, None))
    }

    /// Returns an [AlloyChainProvider] from the configured provider endpoints.
    pub fn l1_chain_provider(&self) -> AlloyChainProvider {
        AlloyChainProvider::new_http(self.l1_rpc_url.clone())
    }

    /// Returns the L2 provider.
    pub fn l2_provider(&self) -> AlloyL2ChainProvider {
        AlloyL2ChainProvider::new_http(
            self.l2_rpc_url.clone(),
            Arc::new(self.rollup_config.clone()),
        )
    }

    /// Returns the safe head tip.
    /// The chain tip includes the safe L1 block info and the L2 block info.
    pub async fn safe_tip(&self) -> Result<(BlockInfo, L2BlockInfo), ConfigError> {
        let mut l2_provider = self.l2_provider();
        let latest_block_number = l2_provider
            .latest_block_number()
            .await
            .map_err(|e| ConfigError::L2ChainProvider(e.to_string()))?;
        let l2_block_info = l2_provider
            .l2_block_info_by_number(latest_block_number)
            .await
            .map_err(|e| ConfigError::L2ChainProvider(e.to_string()))?;

        let mut l1_provider = self.l1_chain_provider();
        let l1_block_info = l1_provider
            .block_info_by_number(l2_block_info.l1_origin.number)
            .await
            .map_err(|e| ConfigError::ChainProvider(e.to_string()))?;

        Ok((l1_block_info, l2_block_info))
    }

    /// Constructs a [PipelineCursor] from the origin.
    pub async fn tip_cursor(&self) -> Result<PipelineCursor, ConfigError> {
        // Load the safe head info.
        let (origin, safe_head_info) = self.safe_tip().await?;

        // Calculate the channel timeout
        let channel_timeout =
            self.rollup_config.channel_timeout(safe_head_info.block_info.timestamp);
        let mut l1_origin_number = origin.number.saturating_sub(channel_timeout);
        if l1_origin_number < self.rollup_config.genesis.l1.number {
            l1_origin_number = self.rollup_config.genesis.l1.number;
        }

        // Create the pipeline cursor from the origin
        let mut l1_provider = self.l1_chain_provider();
        let l1_origin = l1_provider
            .block_info_by_number(l1_origin_number)
            .await
            .map_err(|e| ConfigError::ChainProvider(e.to_string()))?;
        let mut cursor = PipelineCursor::new(channel_timeout, l1_origin);
        // TODO: construct a valid tip cursor
        let tip =
            kona_driver::TipCursor::new(safe_head_info, Default::default(), Default::default());
        cursor.advance(l1_origin, tip);
        Ok(cursor)
    }
}
