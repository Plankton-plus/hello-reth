//! 单元测试夹具；历史读取必须显式配置，不能伪造缺失历史。

use crate::scheme::SchemeReader;
use std::sync::Arc;

/// 创建不提供历史的单块测试读取器；意外读取历史会使测试失败。
pub fn reader() -> SchemeReader {
    SchemeReader::new(
        Arc::new(|_| Err("测试未配置历史头".into())),
        Arc::new(|_| Err("测试未配置历史块".into())),
    )
}

/// 返回带真实日志区间的微型元数据，供解析和原生区块测试共用。
pub fn line() -> &'static str {
    r#"{"lid":"LG_1","fid":"F1","ts_min":100,"ts_max":110,"fw":["FTP","incorrect_pwd"],"bm_ew":"0x01","rh":[]}"#
}

/// 创建单个数据交易的原生块；签名只用于编码，不执行账户转账。
pub fn block() -> crate::node::block::IndexedBlock {
    use alloy_consensus::{SignableTransaction, TxLegacy};
    use alloy_primitives::{Signature, TxKind, U256};
    let mut input = crate::model::record::RECORD_TAG.to_vec();
    input.extend(line().as_bytes());
    let transaction = TxLegacy {
        to: TxKind::Call(crate::model::record::DATA_ADDRESS),
        input: input.into(),
        ..Default::default()
    }
    .into_signed(Signature::new(U256::from(1), U256::from(1), false));
    let mut ethereum = reth_ethereum::Block::default();
    ethereum.header.number = 1;
    ethereum.body.transactions.push(transaction.into());
    crate::node::block::IndexedBlock::from_ethereum(ethereum, &reader()).unwrap()
}
