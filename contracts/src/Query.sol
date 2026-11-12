// SPDX-License-Identifier: MIT
// Argus 查询入口；完整索引由节点保存，合约通过只读预编译查询。
pragma solidity 0.8.30;

contract Query {
    address private constant PRECOMPILE = address(0xa901);
    error QueryError(string reason);

    struct QueryRequest {
        bytes32 endBlockHash;
        uint64 startBlock;
        uint64 timeMin;
        uint64 timeMax;
        string[] generators;
        bytes predicate;
    }

    struct QueryMatch {
        bytes32 blockHash;
        uint32 transactionIndex;
        bytes32 transactionHash;
        bytes rawTransaction;
    }

    /// @notice 查询历史日志元数据；时间按闭区间相交，谓词支持 AND、OR、NOT。
    /// @param request 快照范围、日志时间、生成器集合及 JSON 布尔表达式字节。
    /// @return matches 按块高和交易位置排序，包含原始签名交易。
    function query(QueryRequest calldata request) external view returns (QueryMatch[] memory matches) {
        (bool ok, bytes memory data) = PRECOMPILE.staticcall(abi.encode(request));
        if (!ok) {
            // 原样传递节点的 QueryError 或耗气失败，不能返回伪空结果。
            assembly { revert(add(data, 32), mload(data)) }
        }
        return abi.decode(data, (QueryMatch[]));
    }
}
