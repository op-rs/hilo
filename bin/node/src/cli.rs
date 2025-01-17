//! CLI arguments for the Hilo Node.

use std::{fs::File, path::PathBuf};

use clap::Parser;
use eyre::{bail, Context, Result};
use serde_json::from_reader;
use tracing::debug;
use url::Url;

use alloy_rpc_types_engine::JwtSecret;
use maili_genesis::RollupConfig;
use maili_registry::ROLLUP_CONFIGS;

use hilo_engine::ValidationMode;
use hilo_node::SyncMode;

/// The default L2 chain ID to use. This corresponds to OP Mainnet.
pub const DEFAULT_L2_CHAIN_ID: u64 = 10;

/// The default L1 RPC URL to use.
pub const DEFAULT_L1_RPC_URL: &str = "https://cloudflare-eth.com";

/// The default L2 RPC URL to use.
pub const DEFAULT_L2_RPC_URL: &str = "https://optimism.llamarpc.com/";

/// The default L1 Beacon Client RPC URL to use.
pub const DEFAULT_L1_BEACON_CLIENT_URL: &str = "http://localhost:5052/";

/// CLI Arguments.
#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct NodeArgs {
    /// A port to serve prometheus metrics on.
    #[clap(long, default_value = "9090", help = "The port to serve prometheus metrics on")]
    pub metrics_port: u16,

    /// URL of the checkpoint sync server to fetch checkpoints from.
    #[clap(long = "checkpoint-sync-url")]
    pub checkpoint_sync_url: Option<Url>,

    /// An RPC URL to run the node's rpc server.
    #[clap(long = "rpc-url")]
    pub rpc_url: Option<Url>,

    /// Chain ID of the L2 network
    #[clap(long = "l2-chain-id", default_value_t = DEFAULT_L2_CHAIN_ID)]
    pub l2_chain_id: u64,

    /// Path to a custom L2 rollup configuration file
    /// (overrides the default rollup configuration from the registry)
    #[clap(long = "l2-config-file")]
    pub l2_config_file: Option<PathBuf>,

    /// RPC URL of an L1 execution client
    /// (This is only needed when running in Standalone mode)
    #[clap(long = "l1-rpc-url", default_value = DEFAULT_L1_RPC_URL)]
    pub l1_rpc_url: Url,

    /// URL of an L1 beacon client to fetch blobs
    #[clap(long = "l1-beacon-client-url", default_value = DEFAULT_L1_BEACON_CLIENT_URL)]
    pub l1_beacon_client_url: Url,

    /// RPC URL of an L2 execution client
    #[clap(long = "l2-rpc-url", default_value = DEFAULT_L2_RPC_URL)]
    pub l2_rpc_url: Url,

    #[clap(short = 'm', long, default_value = "full")]
    pub sync_mode: SyncMode,

    /// URL of the blob archiver to fetch blobs that are expired on
    /// the beacon client but still needed for processing.
    ///
    /// Blob archivers need to implement the `blob_sidecars` API:
    /// <https://ethereum.github.io/beacon-APIs/#/Beacon/getBlobSidecars>
    #[clap(long = "l1-blob-archiver-url")]
    pub l1_blob_archiver_url: Option<Url>,

    /// The payload validation mode to use.
    ///
    /// - Trusted: rely on a trusted synced L2 execution client. Validation happens by fetching the
    ///   same block and comparing the results.
    /// - Engine API: use a local or remote engine API of an L2 execution client. Validation
    ///   happens by sending the `new_payload` to the API and expecting a VALID response.
    #[clap(long = "validation-mode", default_value = "engine-api")]
    pub validation_mode: ValidationMode,

    /// URL of the engine API endpoint of an L2 execution client.
    #[clap(long = "l2-engine-api-url", env = "L2_ENGINE_API_URL")]
    pub l2_engine_api_url: Url,

    /// JWT secret for the auth-rpc endpoint of the execution client.
    /// This MUST be a valid path to a file containing the hex-encoded JWT secret.
    #[clap(long = "l2-engine-jwt-secret", env = "L2_ENGINE_JWT_SECRET")]
    pub l2_engine_jwt_secret: Option<PathBuf>,

    /// The maximum **number of blocks** to keep cached in the chain provider.
    ///
    /// This is used to limit the memory usage of the chain provider.
    /// When the limit is reached, the oldest blocks are discarded.
    #[clap(long = "l1-chain-cache-size", default_value_t = 256)]
    pub l1_chain_cache_size: usize,
}

#[allow(unused)]
impl NodeArgs {
    /// Get the L2 rollup config, either from a file or the superchain registry.
    pub fn get_l2_config(&self) -> Result<RollupConfig> {
        match &self.l2_config_file {
            Some(path) => {
                debug!("Loading l2 config from file: {:?}", path);
                let file = File::open(path).wrap_err("Failed to open l2 config file")?;
                Ok(from_reader(file).wrap_err("Failed to read l2 config file")?)
            }
            None => {
                debug!("Loading l2 config from superchain registry");
                let Some(cfg) = ROLLUP_CONFIGS.get(&self.l2_chain_id).cloned() else {
                    bail!("Failed to find l2 config for chain ID {}", self.l2_chain_id);
                };
                Ok(cfg)
            }
        }
    }

    /// Returns the JWT secret for the engine API
    /// using the provided [PathBuf]. If the file is not found,
    /// it will return the default JWT secret.
    pub fn jwt_secret(&self) -> Option<JwtSecret> {
        if let Some(path) = &self.l2_engine_jwt_secret {
            if let Ok(secret) = std::fs::read_to_string(path) {
                return JwtSecret::from_hex(secret).ok();
            }
        }
        Self::default_jwt_secret()
    }

    /// Uses the current directory to attempt to read
    /// the JWT secret from a file named `jwt.hex`.
    /// If the file is not found, it will return `None`.
    pub fn default_jwt_secret() -> Option<JwtSecret> {
        let cur_dir = std::env::current_dir().ok()?;
        match std::fs::read_to_string(cur_dir.join("jwt.hex")) {
            Ok(content) => JwtSecret::from_hex(content).ok(),
            Err(_) => {
                use std::io::Write;
                let secret = JwtSecret::random();
                if let Ok(mut file) = File::create("jwt.hex") {
                    if let Err(e) =
                        file.write_all(alloy_primitives::hex::encode(secret.as_bytes()).as_bytes())
                    {
                        tracing::error!("Failed to write JWT secret to file: {:?}", e);
                    }
                }
                Some(secret)
            }
        }
    }
}

impl From<NodeArgs> for hilo_node::Config {
    fn from(args: NodeArgs) -> Self {
        let rollup_config = args.get_l2_config().unwrap();
        let jwt_secret = args.jwt_secret().unwrap();
        Self {
            l2_chain_id: args.l2_chain_id,
            l1_rpc_url: args.l1_rpc_url,
            l1_beacon_url: args.l1_beacon_client_url,
            blob_archiver_url: args.l1_blob_archiver_url,
            l2_rpc_url: args.l2_rpc_url,
            l2_engine_url: args.l2_engine_api_url,
            rollup_config,
            jwt_secret,
            checkpoint_sync_url: args.checkpoint_sync_url,
            sync_mode: args.sync_mode,
            rpc_url: args.rpc_url,
            devnet: false,
            cache_size: args.l1_chain_cache_size,
        }
    }
}
