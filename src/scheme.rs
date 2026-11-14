//! 方案接口和当前方案选择

use crate::{
    chain::BuildContext,
    model::{metadata::IndexedRecord, query::LogQuery},
    query::{
        abi::QueryResponse,
        context::{QueryContext, QueryFailure, WorkBudget},
    },
};
use alloy_rlp::{Decodable, Encodable};
use serde::{Serialize, de::DeserializeOwned};
use std::{fmt::Debug, hash::Hash};

/// 每条实验链固定一个方案；方案变更使用独立数据库。
pub type ActiveScheme = crate::schemes::blas::BlasScheme;
pub type HeaderFields = <ActiveScheme as Scheme>::HeaderFields;
pub type BodyFields = <ActiveScheme as Scheme>::BodyFields;
pub type SchemeReader = crate::chain::ChainReader<HeaderFields, BodyFields>;

pub trait Scheme: Sized {
    const ID: u16; //方案编号
    const VERSION: u16; //方案版本
    /// 方案自己的原生区块头字段。
    type HeaderFields: Clone
        + Debug
        + Default
        + Eq
        + Hash
        + Serialize
        + DeserializeOwned
        + Encodable
        + Decodable
        + Send
        + Sync
        + Unpin
        + 'static;
    /// 方案产生的完整区块体扩展数据。
    type BodyFields: Clone
        + Debug
        + Default
        + Eq
        + Serialize
        + DeserializeOwned
        + Encodable
        + Decodable
        + Send
        + Sync
        + Unpin
        + 'static;

    /// 在 context 的父链上，从同一 records 输入构建头字段和完整索引。
    fn build(
        context: &BuildContext<'_, Self::HeaderFields, Self::BodyFields>,
        records: &[IndexedRecord],
    ) -> Result<(Self::HeaderFields, Self::BodyFields), String>;

    /// 检查 header 对 body 的认证关系；读取已接受区块时使用。
    fn verify_commitment(
        header: &Self::HeaderFields,
        body: &Self::BodyFields,
    ) -> Result<(), String>;

    /// 接收区块时验证 records 与头体字段；校验算法由方案决定。
    fn validate(
        context: &BuildContext<'_, Self::HeaderFields, Self::BodyFields>,
        records: &[IndexedRecord],
        header: &Self::HeaderFields,
        body: &Self::BodyFields,
    ) -> Result<(), String>;

    /// 在 context 的固定分支内执行 query，负责块间和块内查询并遵守 budget。
    fn query(
        context: &QueryContext<'_, Self::HeaderFields, Self::BodyFields>,
        query: &LogQuery,
        budget: &mut dyn WorkBudget,
    ) -> Result<QueryResponse, QueryFailure>;
}
