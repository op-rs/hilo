//! Contains the engine controller.
//!
//! See: <https://github.com/ethereum-optimism/optimism/blob/develop/op-node/rollup/engine/engine_controller.go#L46>

use alloy_consensus::{Header, Sealed};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV2, ExecutionPayloadFieldV2, ExecutionPayloadV2, ForkchoiceState,
    JwtSecret, PayloadStatusEnum,
};
use async_trait::async_trait;
use hilo_providers_alloy::AlloyL2ChainProvider;
use kona_driver::Executor;
use maili_genesis::RollupConfig;
use maili_protocol::{BatchValidationProvider, BlockInfo};
use op_alloy_consensus::OpBlock;
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use std::{sync::Arc, time::Duration};
use tokio::time::sleep;
use url::Url;

use crate::{Engine, EngineClient, EngineControllerError, Epoch};

/// The engine controller.
#[derive(Debug, Clone)]
pub struct EngineController {
    /// The inner engine client which implements [crate::Engine].
    pub client: EngineClient,
    /// An L2 chain provider used to query the full blocks.
    pub provider: AlloyL2ChainProvider,
    /// Blocktime of the L2 chain
    pub blocktime: u64,
    /// Most recent block found on the p2p network
    pub unsafe_head: BlockInfo,
    /// Most recent block that can be derived from L1 data
    pub safe_head: BlockInfo,
    /// Batch epoch of the safe head
    pub safe_epoch: Epoch,
    /// Most recent block that can be derived from finalized L1 data
    pub finalized_head: BlockInfo,
    /// Batch epoch of the finalized head
    pub finalized_epoch: Epoch,
    /// The ecotone timestamp used for fork choice
    pub ecotone_timestamp: Option<u64>,
    /// The canyon timestamp used for fork choice
    pub canyon_timestamp: Option<u64>,
}

impl EngineController {
    /// Creates a new engine controller.
    pub fn new(
        l2_engine_url: Url,
        l2_rpc_url: Url,
        jwt_secret: JwtSecret,
        finalized_head: BlockInfo,
        finalized_epoch: Epoch,
        config: &RollupConfig,
    ) -> Self {
        let client = EngineClient::new_http(
            l2_engine_url.clone(),
            l2_rpc_url.clone(),
            Arc::new(config.clone()),
            jwt_secret,
        );
        let provider = AlloyL2ChainProvider::new_http(l2_rpc_url, Arc::new(config.clone()));
        Self {
            blocktime: config.block_time,
            unsafe_head: finalized_head,
            safe_head: finalized_head,
            safe_epoch: finalized_epoch,
            finalized_head,
            finalized_epoch,
            client,
            provider,
            ecotone_timestamp: config.ecotone_time,
            canyon_timestamp: config.canyon_time,
        }
    }

    /// Instructs the engine to create a block and updates the forkchoice, based on a payload
    /// received via p2p gossip.
    pub async fn handle_unsafe_payload(
        &mut self,
        payload: &ExecutionPayloadEnvelopeV2,
    ) -> Result<(), EngineControllerError> {
        let withdrawals = match &payload.execution_payload {
            ExecutionPayloadFieldV2::V2(ExecutionPayloadV2 { withdrawals, .. }) => {
                withdrawals.clone()
            }
            ExecutionPayloadFieldV2::V1(_) => vec![],
        };
        let payload_inner = payload.clone().into_v1_payload();
        let exec_payload = ExecutionPayloadV2 { payload_inner, withdrawals };
        let status = self.client.new_payload_v2(exec_payload).await?;
        if !status.is_valid() && status.status != PayloadStatusEnum::Accepted {
            return Err(EngineControllerError::InvalidPayloadAttributes);
        }

        let payload = payload.clone().into_v1_payload();
        self.unsafe_head = BlockInfo {
            number: payload.block_number,
            hash: payload.block_hash,
            parent_hash: payload.parent_hash,
            timestamp: payload.timestamp,
        };
        self.update_forkchoice().await?;

        tracing::info!("head updated: {} {:?}", self.unsafe_head.number, self.unsafe_head.hash,);

        Ok(())
    }

    /// Updates the [EngineController] finalized head & epoch
    pub fn update_finalized(&mut self, head: BlockInfo, epoch: Epoch) {
        self.finalized_head = head;
        self.finalized_epoch = epoch;
    }

    /// Sets the [EngineController] unsafe & safe heads, and safe epoch to the current finalized
    /// head & epoch.
    pub fn reorg(&mut self) {
        self.unsafe_head = self.finalized_head;
        self.safe_head = self.finalized_head;
        self.safe_epoch = self.finalized_epoch;
    }

    /// Sends a `ForkchoiceUpdated` message to check if the [Engine] is ready.
    pub async fn engine_ready(&self) -> bool {
        let forkchoice = self.create_forkchoice_state();
        self.client.forkchoice_update(forkchoice, None).await.is_ok()
    }

    /// Updates the forkchoice by sending `engine_forkchoiceUpdatedV2` (v3 post Ecotone) to the
    /// engine with no payload.
    async fn skip_attributes(
        &mut self,
        _attributes: OpPayloadAttributes,
        block: OpBlock,
    ) -> Result<(), EngineControllerError> {
        // let new_epoch = *attributes.payload_attributes.epoch.as_ref().unwrap();
        let new_head = BlockInfo::from(block);
        let new_epoch = new_head.into();
        self.update_safe_head(new_head, new_epoch, false);
        self.update_forkchoice().await?;

        Ok(())
    }

    /// Initiates validation & production of a new block:
    /// - Sends the [OpPayloadAttributes] to the engine via `engine_forkchoiceUpdatedV2` (V3 post
    ///   Ecotone) and retrieves the [ExecutionPayloadEnvelopeV2]
    /// - Executes the [ExecutionPayloadEnvelopeV2] to create a block via `engine_newPayloadV2` (V3
    ///   post Ecotone)
    /// - Updates the [EngineController] `safe_head`, `safe_epoch`, and `unsafe_head`
    /// - Updates the forkchoice and sends this to the engine via `engine_forkchoiceUpdatedV2` (v3
    ///   post Ecotone)
    async fn process_attributes(
        &mut self,
        attributes: OpPayloadAttributes,
    ) -> Result<(), EngineControllerError> {
        let forkchoice = self.create_forkchoice_state();
        let new_head = self.client.accept_payload(forkchoice, attributes).await?;
        let new_epoch = new_head.into();
        self.update_safe_head(new_head, new_epoch, true);
        self.update_forkchoice().await?;
        Ok(())
    }

    /// Sends a `ForkChoiceUpdated` message to the [Engine] with the current `Forkchoice State` and
    /// no payload.
    async fn update_forkchoice(&self) -> Result<(), EngineControllerError> {
        let forkchoice = self.create_forkchoice_state();

        let update = self.client.forkchoice_update(forkchoice, None).await?;
        if !update.payload_status.is_valid() {
            return Err(EngineControllerError::ForkchoiceRejected(update.payload_status));
        }

        Ok(())
    }

    /// Updates the current `safe_head` & `safe_epoch`.
    ///
    /// Also updates the current `unsafe_head` to the given `new_head` if `reorg_unsafe` is `true`,
    /// or if the updated `safe_head` is newer than the current `unsafe_head`
    fn update_safe_head(&mut self, new_head: BlockInfo, new_epoch: Epoch, reorg_unsafe: bool) {
        if self.safe_head != new_head {
            self.safe_head = new_head;
            self.safe_epoch = new_epoch;
        }

        if reorg_unsafe || self.safe_head.number > self.unsafe_head.number {
            self.unsafe_head = new_head;
        }
    }

    /// Fetches the L2 block for a given timestamp from the L2 Execution Client
    async fn block_at(&mut self, timestamp: u64) -> Option<OpBlock> {
        let time_diff = timestamp as i64 - self.finalized_head.timestamp as i64;
        let blocks = time_diff / self.blocktime as i64;
        let block_num = self.finalized_head.number as i64 + blocks;
        self.provider.block_by_number(block_num as u64).await.ok()
    }

    /// Creates a [ForkchoiceState]:
    /// - `head_block` = `unsafe_head`
    /// - `safe_block` = `safe_head`
    /// - `finalized_block` = `finalized_head`
    pub fn create_forkchoice_state(&self) -> ForkchoiceState {
        ForkchoiceState {
            head_block_hash: self.unsafe_head.hash,
            safe_block_hash: self.safe_head.hash,
            finalized_block_hash: self.finalized_head.hash,
        }
    }
}

#[async_trait]
impl Executor for EngineController {
    type Error = EngineControllerError;

    /// Waits for the engine to be ready.
    async fn wait_until_ready(&mut self) {
        let forkchoice = self.create_forkchoice_state();
        while self.client.forkchoice_update(forkchoice, None).await.is_err() {
            sleep(Duration::from_secs(1)).await;
        }
    }

    /// Updates the safe head.
    fn update_safe_head(&mut self, header: Sealed<Header>) {
        if self.safe_head.number < header.number {
            self.safe_head = BlockInfo {
                number: header.number,
                hash: header.hash_slow(),
                timestamp: header.timestamp,
                parent_hash: header.parent_hash,
            };
            self.safe_epoch = self.safe_head.into();
        }

        if header.number > self.unsafe_head.number {
            self.unsafe_head = BlockInfo {
                number: header.number,
                hash: header.hash_slow(),
                timestamp: header.timestamp,
                parent_hash: header.parent_hash,
            };
        }
    }

    /// Receives payload attributes from the driver and handles them.
    async fn execute_payload(
        &mut self,
        attributes: OpPayloadAttributes,
    ) -> Result<Header, EngineControllerError> {
        let block: Option<OpBlock> = self.block_at(attributes.payload_attributes.timestamp).await;

        if let Some(block) = block {
            if should_skip(&block, &attributes) {
                self.skip_attributes(attributes, block).await?;
            } else {
                self.unsafe_head = self.safe_head;
                self.process_attributes(attributes).await?;
            }
        } else {
            self.process_attributes(attributes).await?;
        }

        // Fetch the header from the block.
        let block = self
            .provider
            .block_by_number(self.unsafe_head.number)
            .await
            .map_err(|_| EngineControllerError::BlockFetchFailed(self.unsafe_head.number))?;
        Ok(block.header)
    }

    /// Computes the output root.
    fn compute_output_root(&mut self) -> Result<B256, Self::Error> {
        unimplemented!("Output root computation is not used by hilo")
    }
}

/// True if transactions in [OpPayloadAttributes] are not the same as those in a fetched L2
/// [OpBlock]
fn should_skip(block: &OpBlock, attributes: &OpPayloadAttributes) -> bool {
    use alloy_eips::eip2718::Encodable2718;

    let attributes_hashes = attributes
        .transactions
        .as_ref()
        .unwrap()
        .iter()
        .map(|tx| alloy_primitives::keccak256(&tx.0))
        .collect::<Vec<_>>();

    let block_hashes =
        block.body.transactions.iter().map(|tx| tx.clone().seal().hash()).collect::<Vec<_>>();

    attributes_hashes == block_hashes
        && attributes.payload_attributes.timestamp == block.header.timestamp
        && attributes.payload_attributes.prev_randao == block.header.mix_hash
        && attributes.payload_attributes.suggested_fee_recipient == block.header.beneficiary
        && attributes.gas_limit.map_or(true, |g| block.header.gas_limit == g)
}
