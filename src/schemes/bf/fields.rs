//! BF 方案的原生头字段和复合体字段；两个索引分别认证。

use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct BfHeaderFields {
    pub min_tx_time: u64,
    pub max_tx_time: u64,
    pub record_count: u64,
    pub roots: Vec<B256>,
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct GeneratorEntry {
    pub lid: String,
    pub positions: Vec<u32>,
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct BfBodyFields {
    pub keyword_tree: Bytes,
    pub generator_index: Vec<GeneratorEntry>,
}
