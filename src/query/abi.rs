//! Solidity 与 Rust 共用的 ABI：物理快照范围和统一业务查询。

use alloy_sol_types::sol;

sol! {
    error QueryError(string reason);

    #[derive(Debug)]
    struct QueryRequest {
        bytes32 endBlockHash;
        uint64 startBlock;
        uint64 timeMin;
        uint64 timeMax;
        uint32[] generators;
        bytes predicate;
        bool aggregate;
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct QueryMatch {
        bytes32 blockHash;
        uint32 transactionIndex;
        bytes32 transactionHash;
        bytes rawTransaction;
    }

    #[derive(Debug)]
    struct QueryResponse {
        QueryMatch[] matches;
        bytes proof;
    }
}
