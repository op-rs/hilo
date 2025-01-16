//! Contains the core `HiloDriver`.

use alloy_provider::ReqwestProvider;
use alloy_transport::TransportResult;
use kona_derive::{errors::PipelineErrorKind, traits::SignalReceiver, types::ResetSignal};
use kona_driver::{Driver, PipelineCursor, TipCursor};
use std::sync::Arc;

use hilo_engine::EngineController;
use hilo_providers_alloy::AlloyL2ChainProvider;
use hilo_providers_local::InMemoryChainProvider;

use crate::{
    ChainNotification, Config, ConfigError, Context, HiloDerivationPipeline, HiloPipeline,
    StandaloneContext,
};

/// A driver from [kona_driver] that uses hilo-types.
pub type KonaDriver = Driver<EngineController, HiloPipeline, HiloDerivationPipeline>;

/// An error that can happen when running the driver.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// An error thrown from a method on the [Config].
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    /// A pipeline reset failed.
    #[error("pipeline reset error: {0}")]
    PipelineReset(#[from] PipelineErrorKind),
    /// Kona's driver unexpectedly errored.
    #[error("kona driver error")]
    DriverErrored,
    /// Shutdown signal received.
    #[error("shutdown signal received")]
    Shutdown,
}

/// HiloDriver is a wrapper around the `Driver` that
/// provides methods of constructing the driver.
#[derive(Debug)]
pub struct HiloDriver<C: Context> {
    /// The driver context.
    pub ctx: C,
    /// The driver config.
    pub cfg: Config,
}

impl HiloDriver<StandaloneContext> {
    /// Creates a new [HiloDriver] with a standalone context.
    pub async fn standalone(cfg: Config) -> TransportResult<Self> {
        let ctx = StandaloneContext::new(cfg.l1_rpc_url.clone()).await?;
        Ok(Self::new(cfg, ctx))
    }
}

impl<C> HiloDriver<C>
where
    C: Context,
{
    /// Constructs a new [HiloDriver].
    pub fn new(cfg: Config, ctx: C) -> Self {
        Self { cfg, ctx }
    }

    /// Initializes the [HiloPipeline].
    pub async fn init_pipeline(&self, cursor: PipelineCursor) -> Result<HiloPipeline, ConfigError> {
        let chain_provider = InMemoryChainProvider::with_capacity(self.cfg.cache_size);
        // let l2_chain_provider = InMemoryL2ChainProvider::with_capacity(self.cfg.cache_size);
        let provider = ReqwestProvider::new_http(self.cfg.l2_rpc_url.clone());
        let l2_chain_provider =
            AlloyL2ChainProvider::new(provider, Arc::new(self.cfg.rollup_config.clone()));
        Ok(HiloPipeline::new(
            Arc::new(self.cfg.rollup_config.clone()),
            cursor,
            self.cfg.blob_provider().await?,
            chain_provider.clone(),
            l2_chain_provider,
        ))
    }

    /// Initializes a [Driver] using the [HiloPipeline].
    pub async fn init_driver(&mut self) -> Result<KonaDriver, ConfigError> {
        let cursor = self.cfg.tip_cursor().await?;
        let pipeline = self.init_pipeline(cursor.clone()).await?;
        let exec = EngineController::new(
            self.cfg.l2_engine_url.clone(),
            self.cfg.l2_rpc_url.clone(),
            self.cfg.jwt_secret,
            cursor.origin(),
            cursor.l2_safe_head().block_info.into(),
            &self.cfg.rollup_config,
        );
        Ok(Driver::new(cursor, exec, pipeline))
    }

    /// Handle a chain notification from the driver context.
    async fn handle_notification(
        &mut self,
        notification: ChainNotification,
        driver: &mut KonaDriver,
    ) -> Result<(), DriverError> {
        if let Some(reverted_chain) = notification.reverted_chain() {
            // The reverted chain contains the list of blocks that were invalidated by the
            // reorg. we need to reset the cursor to the last canonical block, which corresponds
            // to the block before the reorg happened.
            let fork_block = reverted_chain.fork_block_number();

            // Find the last known L2 block that is still valid after the reorg,
            // and reset the cursor and pipeline to it.
            let (TipCursor { l2_safe_head, .. }, l1_origin) = driver.cursor.reset(fork_block);

            warn!("Resetting derivation pipeline to L2 block: {}", l2_safe_head.block_info.number);
            let reset_signal = ResetSignal { l1_origin, l2_safe_head, ..Default::default() };
            if let Err(e) = driver.pipeline.signal(reset_signal.signal()).await {
                return Err(DriverError::PipelineReset(e));
            }
        }

        if let Some(new_chain) = notification.new_chain() {
            let tip = new_chain.tip();
            self.ctx.send_processed_tip_event(tip);
        }

        Ok(())
    }

    /// Continuously run the [HiloDriver].
    pub async fn start(&mut self) -> Result<(), DriverError> {
        // Step 1: Wait for the L2 origin block to be available
        self.wait_for_l2_genesis_l1_block().await;
        info!("L1 chain synced to the rollup genesis block");

        // Step 2: Initialize the kona driver
        let mut driver = self.init_driver().await?;
        info!("Driver initialized");

        // Wait until the engine is ready
        driver.wait_for_executor().await;

        // Step 3: Start the processing loop
        loop {
            tokio::select! {
                result = driver.advance_to_target(&self.cfg.rollup_config, None) => match result {
                    Ok((bn, hash, _output_root)) => {
                        error!("Driver unexpectedly stopped at target block: {} {}", bn, hash);
                    }
                    Err(e) => {
                        error!("Driver error: {}", e);
                        // TODO: optionally allow recovery
                        return Err(DriverError::DriverErrored);
                    }
                },
                Some(notification) = self.ctx.recv_notification() => {
                    self.handle_notification(notification, &mut driver).await?;
                }
            }
        }
    }

    // Exits if a SIGINT signal is received
    // fn check_shutdown(&self) -> Result<(), DriverError> {
    //     if *self.shutdown_recv.borrow() {
    //         tracing::warn!("shutting down");
    //         std::process::exit(1);
    //     }
    //
    //     Ok(())
    // }

    /// Wait for the L2 genesis' corresponding L1 block to be available in the L1 chain.
    async fn wait_for_l2_genesis_l1_block(&mut self) {
        loop {
            if let Some(notification) = self.ctx.recv_notification().await {
                if let Some(new_chain) = notification.new_chain() {
                    let tip = new_chain.tip();
                    self.ctx.send_processed_tip_event(tip);

                    if tip.number >= self.cfg.rollup_config.genesis.l1.number {
                        break;
                    }
                    debug!(
                        "Chain not yet synced to rollup genesis. L1 block number: {}",
                        tip.number
                    );
                }
            }
        }
    }
}
