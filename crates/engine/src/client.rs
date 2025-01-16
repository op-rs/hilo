//! Contains the engine api client.

use alloy_eips::eip1898::BlockNumberOrTag;
use alloy_network::AnyNetwork;
use alloy_primitives::{Bytes, B256};
use alloy_provider::{ReqwestProvider, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV2, ExecutionPayloadFieldV2, ExecutionPayloadInputV2,
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated,
    JwtSecret, PayloadId, PayloadStatus, PayloadStatusEnum,
};
use alloy_transport_http::{
    hyper_util::{
        client::legacy::{connect::HttpConnector, Client},
        rt::TokioExecutor,
    },
    AuthLayer, AuthService, Http, HyperClient,
};
use async_trait::async_trait;
use http_body_util::Full;
use maili_genesis::RollupConfig;
use maili_protocol::{BatchValidationProvider, BlockInfo, L2BlockInfo};
use op_alloy_provider::ext::engine::OpEngineApi;
use op_alloy_rpc_types_engine::{OpExecutionPayloadEnvelopeV3, OpPayloadAttributes};
use std::sync::Arc;
use tower::ServiceBuilder;
use url::Url;

use hilo_providers_alloy::AlloyL2ChainProvider;

use crate::{Engine, EngineError};

/// A Hyper HTTP client with a JWT authentication layer.
type HyperAuthClient<B = Full<Bytes>> = HyperClient<B, AuthService<Client<HttpConnector, B>>>;

/// An external engine api client
#[derive(Debug, Clone)]
pub struct EngineClient {
    /// The L2 engine provider.
    engine: RootProvider<Http<HyperAuthClient>, AnyNetwork>,
    /// The L2 chain provider.
    rpc: AlloyL2ChainProvider,
    /// The [RollupConfig] for the chain used to timestamp which version of the engine api to use.
    cfg: Arc<RollupConfig>,
}

impl EngineClient {
    /// Creates a new [`EngineClient`] from the provided [Url] and [JwtSecret].
    pub fn new_http(engine: Url, rpc: Url, cfg: Arc<RollupConfig>, jwt: JwtSecret) -> Self {
        let hyper_client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();

        let auth_layer = AuthLayer::new(jwt);
        let service = ServiceBuilder::new().layer(auth_layer).service(hyper_client);

        let layer_transport = HyperClient::with_service(service);
        let http_hyper = Http::with_client(layer_transport, engine);
        let rpc_client = RpcClient::new(http_hyper, true);
        let engine = RootProvider::<_, AnyNetwork>::new(rpc_client);

        let rpc = ReqwestProvider::new_http(rpc);
        let rpc = AlloyL2ChainProvider::new(rpc, cfg.clone());
        Self { engine, rpc, cfg }
    }

    /// Returns which fork choice version to use based on the timestamp
    /// and rollup config.
    pub fn fork_choice_version(&self, timestamp: u64) -> u64 {
        // TODO: replace this with https://github.com/alloy-rs/maili/pull/321
        //       once it's merged and updated in kona.
        if self.cfg.ecotone_time.is_some_and(|t| timestamp >= t) {
            // Cancun
            3
        } else if self.cfg.canyon_time.is_some_and(|t| timestamp >= t) {
            // Shanghai
            2
        } else {
            1
        }
    }

    /// Accepts a given payload.
    /// Sends [OpPayloadAttributes] via a `ForkChoiceUpdated` message to the [Engine].
    /// If the payload is valid, the engine will create a new block and update the `safe_head`,
    /// `safe_epoch`, and `unsafe_head`.
    pub async fn accept_payload(
        &mut self,
        forkchoice: ForkchoiceState,
        attributes: OpPayloadAttributes,
    ) -> Result<BlockInfo, EngineError> {
        let timestamp = attributes.payload_attributes.timestamp;
        let update = self.forkchoice_update(forkchoice, Some(attributes)).await?;

        if !update.payload_status.status.is_valid() {
            return Err(EngineError::InvalidForkChoiceAttributes);
        }

        let id = update.payload_id.ok_or(EngineError::MissingPayloadId)?;

        match self.fork_choice_version(timestamp) {
            1 => self.accept_v1(id).await,
            2 => self.accept_v2(id).await,
            3 => self.accept_v3(id).await,
            _ => Err(EngineError::InvalidNewPayloadAttributes),
        }
    }

    /// Gets and marks a new payload for the V1 engine api.
    pub async fn accept_v1(&mut self, _id: PayloadId) -> Result<BlockInfo, EngineError> {
        unimplemented!("v1 not supported by OpEngineApi")
    }

    /// Gets and marks a new payload for the V2 engine api.
    pub async fn accept_v2(&mut self, id: PayloadId) -> Result<BlockInfo, EngineError> {
        let payload = self.get_payload_v2(id).await?;

        let withdrawals = match &payload.execution_payload {
            ExecutionPayloadFieldV2::V2(ExecutionPayloadV2 { withdrawals, .. }) => {
                withdrawals.clone()
            }
            ExecutionPayloadFieldV2::V1(_) => vec![],
        };
        let payload_inner = payload.into_v1_payload();
        let block_info = BlockInfo {
            number: payload_inner.block_number,
            hash: payload_inner.block_hash,
            parent_hash: payload_inner.parent_hash,
            timestamp: payload_inner.timestamp,
        };
        let payload = ExecutionPayloadV2 { payload_inner, withdrawals };
        let status = self.new_payload_v2(payload.clone()).await?;
        if !status.is_valid() && status.status != PayloadStatusEnum::Accepted {
            return Err(EngineError::InvalidNewPayloadAttributes);
        }

        Ok(block_info)
    }

    /// Gets and marks a new payload for the V3 engine api.
    pub async fn accept_v3(&mut self, id: PayloadId) -> Result<BlockInfo, EngineError> {
        let payload = self.get_payload_v3(id).await?;

        let block_info = BlockInfo {
            number: payload.execution_payload.payload_inner.payload_inner.block_number,
            hash: payload.execution_payload.payload_inner.payload_inner.block_hash,
            parent_hash: payload.execution_payload.payload_inner.payload_inner.parent_hash,
            timestamp: payload.execution_payload.payload_inner.payload_inner.timestamp,
        };
        let status = self.new_payload_v3(payload.execution_payload, block_info.hash).await?;
        if !status.is_valid() && status.status != PayloadStatusEnum::Accepted {
            return Err(EngineError::InvalidNewPayloadAttributes);
        }

        Ok(block_info)
    }
}

#[async_trait]
impl Engine for EngineClient {
    type Error = EngineError;

    async fn get_payload_v1(
        &self,
        _payload_id: PayloadId,
    ) -> Result<ExecutionPayloadV1, Self::Error> {
        unimplemented!("v1 not supported by OpEngineApi")
    }

    async fn get_payload_v2(
        &self,
        payload_id: PayloadId,
    ) -> Result<ExecutionPayloadEnvelopeV2, Self::Error> {
        self.engine.get_payload_v2(payload_id).await.map_err(|_| EngineError::PayloadError)
    }

    async fn get_payload_v3(
        &self,
        payload_id: PayloadId,
    ) -> Result<OpExecutionPayloadEnvelopeV3, Self::Error> {
        self.engine.get_payload_v3(payload_id).await.map_err(|_| EngineError::PayloadError)
    }

    async fn forkchoice_update(
        &self,
        state: ForkchoiceState,
        attr: Option<OpPayloadAttributes>,
    ) -> Result<ForkchoiceUpdated, Self::Error> {
        self.engine.fork_choice_updated_v2(state, attr).await.map_err(|_| EngineError::PayloadError)
    }

    async fn new_payload_v1(
        &self,
        _payload: ExecutionPayloadV1,
    ) -> Result<PayloadStatus, Self::Error> {
        unimplemented!("v1 not supported by OpEngineApi")
    }

    async fn new_payload_v2(
        &self,
        payload: ExecutionPayloadV2,
    ) -> Result<PayloadStatus, Self::Error> {
        self.engine
            .new_payload_v2(ExecutionPayloadInputV2 {
                execution_payload: payload.payload_inner,
                withdrawals: Some(payload.withdrawals),
            })
            .await
            .map_err(|_| EngineError::PayloadError)
    }

    async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        parent_beacon_block_root: B256,
    ) -> Result<PayloadStatus, Self::Error> {
        self.engine
            .new_payload_v3(payload, parent_beacon_block_root)
            .await
            .map_err(|_| EngineError::PayloadError)
    }

    async fn l2_block_ref_by_label(
        &mut self,
        numtag: BlockNumberOrTag,
    ) -> Result<L2BlockInfo, Self::Error> {
        let number = match numtag {
            BlockNumberOrTag::Number(n) => n,
            BlockNumberOrTag::Latest => {
                self.rpc.latest_block_number().await.map_err(|_| EngineError::LatestBlockNumber)?
            }
            _ => return Err(EngineError::InvalidBlockTag),
        };
        self.rpc.l2_block_info_by_number(number).await.map_err(|_| EngineError::L2BlockInfoFetch)
    }
}

impl std::ops::Deref for EngineClient {
    type Target = RootProvider<Http<HyperAuthClient>, AnyNetwork>;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}
