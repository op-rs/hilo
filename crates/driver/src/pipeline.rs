//! A pipeline

use async_trait::async_trait;
use kona_derive::{
    attributes::StatefulAttributesBuilder,
    errors::PipelineErrorKind,
    pipeline::{DerivationPipeline, PipelineBuilder},
    sources::EthereumDataSource,
    stages::{
        AttributesQueue, BatchProvider, BatchStream, ChannelProvider, ChannelReader, FrameQueue,
        L1Retrieval, L1Traversal,
    },
    traits::{OriginProvider, Pipeline, SignalReceiver},
    types::{PipelineResult, Signal, StepResult},
};
use kona_driver::{DriverPipeline, PipelineCursor};
use maili_genesis::{RollupConfig, SystemConfig};
use maili_protocol::{BlockInfo, L2BlockInfo};
use op_alloy_rpc_types_engine::OpAttributesWithParent;
use std::{boxed::Box, sync::Arc};

use hilo_providers_alloy::{AlloyL2ChainProvider, DurableBlobProvider};
use hilo_providers_local::InMemoryChainProvider;

/// Hilo Derivation Pipeline.
pub type HiloDerivationPipeline =
    DerivationPipeline<HiloAttributesQueue<HiloDataProvider>, AlloyL2ChainProvider>;

/// Hilo Ethereum data source.
pub type HiloDataProvider = EthereumDataSource<InMemoryChainProvider, DurableBlobProvider>;

/// Hilo payload attributes builder for the `AttributesQueue` stage of the derivation
/// pipeline.
pub type HiloAttributesBuilder =
    StatefulAttributesBuilder<InMemoryChainProvider, AlloyL2ChainProvider>;

/// Hilo attributes queue for the derivation pipeline.
pub type HiloAttributesQueue<DAP> = AttributesQueue<
    BatchProvider<
        BatchStream<
            ChannelReader<
                ChannelProvider<FrameQueue<L1Retrieval<DAP, L1Traversal<InMemoryChainProvider>>>>,
            >,
            AlloyL2ChainProvider,
        >,
        AlloyL2ChainProvider,
    >,
    HiloAttributesBuilder,
>;

/// Hilo derivation pipeline.
#[derive(Debug)]
pub struct HiloPipeline {
    /// The internal derivation pipeline.
    pub pipeline: HiloDerivationPipeline,
    /// The chain provider.
    #[allow(unused)]
    pub chain_provider: InMemoryChainProvider,
    /// The L2 chain provider.
    #[allow(unused)]
    pub l2_chain_provider: AlloyL2ChainProvider,
}

impl HiloPipeline {
    /// Constructs a new Hilo derivation pipeline.
    pub fn new(
        cfg: Arc<RollupConfig>,
        sync_start: PipelineCursor,
        blob_provider: DurableBlobProvider,
        chain_provider: InMemoryChainProvider,
        l2_chain_provider: AlloyL2ChainProvider,
    ) -> Self {
        let attributes = StatefulAttributesBuilder::new(
            cfg.clone(),
            l2_chain_provider.clone(),
            chain_provider.clone(),
        );
        let dap = EthereumDataSource::new_from_parts(chain_provider.clone(), blob_provider, &cfg);

        let pipeline = PipelineBuilder::new()
            .rollup_config(cfg)
            .dap_source(dap)
            .l2_chain_provider(l2_chain_provider.clone())
            .chain_provider(chain_provider.clone())
            .builder(attributes)
            .origin(sync_start.origin())
            .build();
        Self { pipeline, chain_provider, l2_chain_provider }
    }
}

impl DriverPipeline<HiloDerivationPipeline> for HiloPipeline {
    /// Flushes provider caches on re-orgs.
    fn flush(&mut self) {
        self.chain_provider.flush();
        // self.l2_chain_provider.flush();
    }
}

#[async_trait]
impl SignalReceiver for HiloPipeline {
    /// Receives a signal from the driver.
    async fn signal(&mut self, signal: Signal) -> PipelineResult<()> {
        self.pipeline.signal(signal).await
    }
}

impl OriginProvider for HiloPipeline {
    /// Returns the optional L1 [BlockInfo] origin.
    fn origin(&self) -> Option<BlockInfo> {
        self.pipeline.origin()
    }
}

impl Iterator for HiloPipeline {
    type Item = OpAttributesWithParent;

    fn next(&mut self) -> Option<Self::Item> {
        self.pipeline.next()
    }
}

#[async_trait]
impl Pipeline for HiloPipeline {
    /// Peeks at the next [OpAttributesWithParent] from the pipeline.
    fn peek(&self) -> Option<&OpAttributesWithParent> {
        self.pipeline.peek()
    }

    /// Attempts to progress the pipeline.
    async fn step(&mut self, cursor: L2BlockInfo) -> StepResult {
        self.pipeline.step(cursor).await
    }

    /// Returns the rollup config.
    fn rollup_config(&self) -> &RollupConfig {
        self.pipeline.rollup_config()
    }

    /// Returns the [SystemConfig] by L2 number.
    async fn system_config_by_number(
        &mut self,
        number: u64,
    ) -> Result<SystemConfig, PipelineErrorKind> {
        self.pipeline.system_config_by_number(number).await
    }
}
