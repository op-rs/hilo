//! Context types for the driver.

use std::{collections::BTreeMap, sync::Arc};

use alloy_eips::eip1898::BlockNumHash;
use alloy_primitives::{BlockNumber, U256};
use alloy_rpc_types_eth::Header;
use reth_execution_types::Chain;
use reth_exex::ExExNotification;

/// A notification representing a chain of blocks that come from an execution client.
#[derive(Debug, Clone)]
pub enum ChainNotification {
    /// A new chain of blocks has been processed.
    New { new_blocks: Headers },
    /// Some blocks have been reverted and are no longer part of the chain.
    Revert { old_blocks: Headers },
    /// The chain has been reorganized with new canonical blocks.
    Reorg { old_blocks: Headers, new_blocks: Headers },
}

impl ChainNotification {
    /// Returns the new chain of blocks contained in the notification event.
    ///
    /// For reorgs, this returns the new chain of blocks that replaced the old one.
    pub fn new_chain(&self) -> Option<Headers> {
        match self {
            ChainNotification::New { new_blocks } => Some(new_blocks.clone()),
            ChainNotification::Reorg { new_blocks, .. } => Some(new_blocks.clone()),
            ChainNotification::Revert { .. } => None,
        }
    }

    /// Returns the old chain of blocks contained in the notification event.
    ///
    /// For reorgs, this returns the old canonical chain that was reorged.
    pub fn reverted_chain(&self) -> Option<Headers> {
        match self {
            ChainNotification::Revert { old_blocks } => Some(old_blocks.clone()),
            ChainNotification::Reorg { old_blocks, .. } => Some(old_blocks.clone()),
            ChainNotification::New { .. } => None,
        }
    }
}

/// A collection of headers that form a chain.
#[derive(Debug, Clone, Default)]
pub struct Headers(Arc<BTreeMap<BlockNumber, Header>>);

impl Headers {
    /// Returns the tip of the chain.
    pub fn tip(&self) -> BlockNumHash {
        let last = self.0.last_key_value().expect("Blocks should have at least one block").1;
        BlockNumHash::new(last.number, last.hash)
    }

    /// Returns the block at the fork point of the chain.
    pub fn fork_block_number(&self) -> BlockNumber {
        let first = self.0.first_key_value().expect("Blocks should have at least one block").0;
        first.saturating_sub(1)
    }
}

impl From<Header> for Headers {
    fn from(value: Header) -> Self {
        let mut headers = BTreeMap::new();
        headers.insert(value.number, value);
        Self(Arc::new(headers))
    }
}

impl From<Vec<Header>> for Headers {
    fn from(value: Vec<Header>) -> Self {
        let mut headers = BTreeMap::new();
        for header in value {
            headers.insert(header.number, header);
        }
        Self(Arc::new(headers))
    }
}

impl From<Arc<Chain>> for Headers {
    fn from(value: Arc<Chain>) -> Self {
        let mut headers = BTreeMap::new();
        for (block_number, sealed_block) in value.blocks() {
            let header = parse_reth_header_to_alloy_rpc(&sealed_block.block);
            headers.insert(*block_number, header);
        }
        Self(Arc::new(headers))
    }
}

impl From<ExExNotification> for ChainNotification {
    fn from(value: ExExNotification) -> Self {
        match value {
            ExExNotification::ChainCommitted { new } => Self::New { new_blocks: new.into() },
            ExExNotification::ChainReverted { old } => Self::Revert { old_blocks: old.into() },
            ExExNotification::ChainReorged { old, new } => {
                Self::Reorg { old_blocks: old.into(), new_blocks: new.into() }
            }
        }
    }
}

// fn parse_reth_block_to_alloy_rpc(block: reth_primitives::SealedBlock) -> Block {
//     let tx_hashes = block.body.transactions().map(|tx| tx.hash()).collect();
//
//     Block {
//         header: parse_reth_header_to_alloy_rpc(&block),
//         uncles: block.body.ommers.iter().map(|x| x.hash_slow()).collect(),
//         transactions: BlockTransactions::Hashes(tx_hashes),
//         withdrawals: block.body.withdrawals.clone().map(|w| w.into_inner()),
//     }
// }

fn parse_reth_header_to_alloy_rpc(block: &reth_primitives::SealedBlock) -> Header {
    Header {
        hash: block.header.hash(),
        size: Some(U256::from(block.size())),
        total_difficulty: Some(block.difficulty),
        inner: alloy_consensus::Header {
            parent_hash: block.parent_hash,
            ommers_hash: block.ommers_hash,
            beneficiary: block.beneficiary,
            state_root: block.state_root,
            transactions_root: block.transactions_root,
            receipts_root: block.receipts_root,
            logs_bloom: block.logs_bloom,
            difficulty: block.difficulty,
            number: block.number,
            gas_limit: block.gas_limit,
            gas_used: block.gas_used,
            timestamp: block.timestamp,
            extra_data: block.extra_data.clone(),
            mix_hash: block.mix_hash,
            nonce: block.nonce,
            base_fee_per_gas: block.base_fee_per_gas,
            withdrawals_root: block.withdrawals_root,
            blob_gas_used: block.blob_gas_used,
            excess_blob_gas: block.excess_blob_gas,
            parent_beacon_block_root: block.parent_beacon_block_root,
            requests_hash: block.requests_root,
        },
    }
}
