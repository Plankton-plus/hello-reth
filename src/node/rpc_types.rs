//! RPC 保留完整自定义头，交易和回执沿用 Ethereum 格式。

use super::header::IndexedHeader;
use reth_ethereum::rpc::eth::primitives::{
    Header, Log, Transaction, TransactionReceipt, TransactionRequest,
};
use reth_rpc_convert::RpcTypes;

#[derive(Clone, Debug)]
pub struct IndexedRpcTypes;

impl RpcTypes for IndexedRpcTypes {
    type Header = Header<IndexedHeader>; //告诉reth的RPC层对外暴露时改用哪些类型
    type Receipt = TransactionReceipt;
    type Log = Log;
    type TransactionResponse = Transaction;
    type TransactionRequest = TransactionRequest;
}
