//! Contains an epoch type.

use alloy_primitives::B256;
use maili_protocol::BlockInfo;

/// L1 epoch block
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Epoch {
    /// The block number
    pub number: u64,
    /// The block hash
    pub hash: B256,
    /// The block timestamp
    pub timestamp: u64,
}

impl From<BlockInfo> for Epoch {
    fn from(block: BlockInfo) -> Self {
        Self { number: block.number, hash: block.hash, timestamp: block.timestamp }
    }
}
