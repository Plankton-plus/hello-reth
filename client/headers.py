"""独立编码原生头并沿父哈希认证查询范围，不读取节点 ADS。"""
from client.rpc import rpc


def rlp(value):
    """按规范编码整数、字节或列表 value，供完整头哈希检查使用。"""
    if isinstance(value, int):
        value = value.to_bytes((value.bit_length() + 7) // 8, "big")
    if isinstance(value, list):
        payload, offset = b"".join(rlp(item) for item in value), 192
    else:
        payload, offset = value, 128
        if len(payload) == 1 and payload[0] < 128:
            return payload
    if len(payload) < 56:
        return bytes([offset + len(payload)]) + payload
    length = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, "big")
    return bytes([offset + 55 + len(length)]) + length + payload


def native_hash(header):
    """重算 header 的全部原生字段哈希，包括 BLAS 根、方案及窗口参数。"""
    from client.query import cast
    names = ["parentHash", "sha3Uncles", "miner", "stateRoot", "transactionsRoot", "receiptsRoot",
             "logsBloom", "difficulty", "number", "gasLimit", "gasUsed", "timestamp", "extraData", "mixHash", "nonce",
             "baseFeePerGas", "withdrawalsRoot", "blobGasUsed", "excessBlobGas", "parentBeaconBlockRoot",
             "requestsHash", "blockAccessListHash", "slotNumber"]
    quantities = {"difficulty", "number", "gasLimit", "gasUsed", "timestamp", "baseFeePerGas",
                  "blobGasUsed", "excessBlobGas", "slotNumber"}
    last = max(index for index, name in enumerate(names) if name in header)
    ethereum = [b"" if name not in header else int(header[name], 16) if name in quantities
                else bytes.fromhex(header[name][2:]) for name in names[:last + 1]]
    fields = header["fields"]
    extension = [fields["min_tx_time"], fields["max_tx_time"], fields["record_count"],
                 [bytes.fromhex(root[2:]) for root in fields["roots"]], fields["window_size"]]
    return cast("keccak", "0x" + rlp([ethereum, header["scheme"], header["version"], extension]).hex())


def authenticated_headers(request):
    """以调用方选定的 endBlockHash 为信任锚，认证至 startBlock 的连续完整头。"""
    headers, cursor = [], request["endBlockHash"]
    for _ in range(32):
        header = rpc("eth_getBlockByHash", [cursor, False])
        assert header and native_hash(header) == cursor, "原生区块头哈希不一致"
        number = int(header["number"], 16)
        assert header["scheme"] == 25 and header["version"] == 1
        headers.append({"hash": cursor, "number": number, "fields": header["fields"]})
        if number == request["startBlock"]:
            headers.reverse()
            assert all(a["number"] + 1 == b["number"] for a, b in zip(headers, headers[1:]))
            return headers
        assert number > request["startBlock"], "查询起点越界"
        cursor = header["parentHash"]
    raise AssertionError("查询跨度超过 32 块")
