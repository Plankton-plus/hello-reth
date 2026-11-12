"""验收 BLAS 完整索引入块、伪造拒绝和重启恢复，结果持久化于状态卷。"""
import copy
import json
import sys
from pathlib import Path

from client.headers import native_hash
from client.rpc import rpc
from scripts.demo import submit_sample
from tests.auth_rpc import auth_rpc

SAVED = Path("/argus/run/body-result.json")


def check_block(block_hash, inputs):
    """检查真实交易位置、业务元数据及标准 RPC 完整头哈希。"""
    block = auth_rpc("argus_getBlock", [block_hash])
    header, index = block["header"], block["body"]["fields"]["index"]
    standard = rpc("eth_getBlockByHash", [block_hash, False])
    positions = [i for i, tx in enumerate(standard["transactions"]) if tx in inputs]
    assert [row["transaction_index"] for row in index["records"]] == positions
    for row, position in zip(index["records"], positions):
        tx = standard["transactions"][position]
        assert row["transaction_hash"] == tx
        assert row["metadata"] == json.loads(bytes.fromhex(inputs[tx][2:])[7:])
    assert header["scheme"] == 25 and header["version"] == 1
    assert header["fields"]["record_count"] == len(positions)
    assert len(header["fields"]["roots"]) == 6 and header["fields"]["window_size"] == 4
    assert standard["fields"] == header["fields"]
    assert native_hash(header) == block_hash
    assert header["fields"]["min_tx_time"] == min((r["metadata"]["t_min"] for r in index["records"]), default=0)
    assert header["fields"]["max_tx_time"] == max((r["metadata"]["t_max"] for r in index["records"]), default=0)
    return block


def check_rejection(blocks, end_hash):
    """篡改单块和窗口索引；换用另一合法索引并重算全头哈希也必须失败。"""
    targets = [(key, value) for key, value in blocks.items() if value["body"]["fields"]["index"]["records"]]
    block_hash, block = targets[0]
    payload = auth_rpc("argus_exportPayload", [block_hash])
    assert auth_rpc("argus_newPayload", [payload])["status"] == "VALID"
    for name in ("record", "missing_tree", "bitmap"):
        forged = copy.deepcopy(payload)
        index = forged["body_fields"]["index"]
        if name == "record":
            index["records"][0]["metadata"]["lid"] = 99
        elif name == "missing_tree":
            index["omt"]["leaves"] = []
        else:
            index["filters"]["keywords"]["leaves"][0]["value"] = "0x00"
        assert auth_rpc("argus_newPayload", [forged])["status"] == "INVALID", name
    other = next(value for key, value in targets if key != block_hash)
    coherent = copy.deepcopy(payload)
    coherent["body_fields"] = other["body"]["fields"]
    coherent["header_fields"] = other["header"]["fields"]
    header = copy.deepcopy(block["header"])
    header["fields"] = coherent["header_fields"]
    coherent["block_hash"] = native_hash(header)
    status = auth_rpc("argus_newPayload", [coherent])
    assert status["status"] == "INVALID" and "索引" in status["validationError"], status
    window = auth_rpc("argus_exportPayload", [end_hash])
    assert window["body_fields"]["windows"], "未实际保存完整窗口索引"
    window["body_fields"]["windows"][0]["blocks"][0]["count"] += 1
    assert auth_rpc("argus_newPayload", [window])["status"] == "INVALID"


def create():
    """提交小样本、验证五类伪造，并保存完整主体用于重启对照。"""
    sample = submit_sample()
    hashes = {row["blockHash"] for row in sample["receipts"]} | {sample["request"]["endBlockHash"]}
    blocks = {key: check_block(key, sample["inputs"]) for key in hashes}
    check_rejection(blocks, sample["request"]["endBlockHash"])
    SAVED.write_text(json.dumps({"blocks": blocks, "transactions": [r["transactionHash"] for r in sample["receipts"]]}, indent=2))
    print(json.dumps({"phase": "create", "status": "PASS", "data_records": 6, "ignored_transactions": 2,
                      "rejected_forgery_types": 5, "blocks": len(blocks)}))


def verify():
    """从已有数据库读回索引，禁止通过重新导入或补建代替持久化验证。"""
    saved = json.loads(SAVED.read_text())
    for block_hash, expected in saved["blocks"].items():
        assert auth_rpc("argus_getBlock", [block_hash]) == expected
    for tx in saved["transactions"]:
        assert rpc("eth_getTransactionReceipt", [tx])["status"] == "0x1"
    print(json.dumps({"phase": "verify", "status": "PASS", "blocks": len(saved["blocks"])}))


if __name__ == "__main__":
    if sys.argv[1:] == ["create"]:
        create()
    elif sys.argv[1:] == ["verify"]:
        verify()
    else:
        raise SystemExit("用法：python3 -m tests.block_body create|verify")
