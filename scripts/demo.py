"""在开发节点提交跨块样本，并用 Forge 部署查询合约。"""

import json
import os
import subprocess
import time
from pathlib import Path

from client.rpc import rpc
from client.transactions import DATA_ADDRESS, wait_receipts
from scripts.data import SAMPLE

DEFAULT_PREDICATE = {"op": "and", "args": [
    {"op": "keyword", "args": "FTP"},
    {"op": "not", "args": {"op": "keyword", "args": "Admin"}},
    {"op": "or", "args": [{"op": "keyword", "args": "incorrect_pwd"},
                            {"op": "keyword", "args": "incorrect_usr"}]}]}
def submit_sample():
    """分三批提交共享元数据，返回请求、原始输入和回执，供部署与验收共用。"""
    sender, other = rpc("eth_accounts", [])[:2]
    lines = SAMPLE.read_text().splitlines()
    assert len(lines) == 6, "请先显式生成六条共享元数据"
    metadata = [json.loads(line) for line in lines]
    payloads = [b"ARGUS2\0" + line.encode() for line in lines]
    receipts, data_hashes, inputs = [], [], {}
    batches = [[(other, payloads[0], False), (DATA_ADDRESS, payloads[0], True), (DATA_ADDRESS, payloads[1], True)],
               [(DATA_ADDRESS, payloads[2], True), (DATA_ADDRESS, payloads[3], True)],
               [(DATA_ADDRESS, payloads[4], True), (DATA_ADDRESS, payloads[5], True), (DATA_ADDRESS, b"ordinary", False)]]
    for batch in batches:
        hashes = []
        for destination, payload, indexed in batch:
            encoded = "0x" + payload.hex()
            tx = rpc("eth_sendTransaction", [{"from": sender, "to": destination, "input": encoded,
                                             "gas": "0x186a0", "gasPrice": "0x3b9aca00"}])
            hashes.append(tx)
            if indexed:
                data_hashes.append(tx)
                inputs[tx] = encoded
        receipts.extend(wait_receipts(hashes))
    numbers = [int(receipt["blockNumber"], 16) for receipt in receipts]
    assert len(set(numbers)) >= 3, "样本未形成三个不同的数据块"
    # 完整四块段让普通与 ALIS 路径都被实际执行，段内空块也必须被覆盖。
    start = (min(numbers) - 1) // 4 * 4 + 1
    end = (max(numbers) + 3) // 4 * 4
    deadline = time.monotonic() + 35
    while int(rpc("eth_blockNumber", []), 16) < end:
        if time.monotonic() > deadline:
            raise AssertionError("等待完整 BLAS 窗口超时")
        time.sleep(0.2)
    end_block = rpc("eth_getBlockByNumber", [hex(end), False])
    request = {"endBlockHash": end_block["hash"], "startBlock": start,
               "timeMin": min(record["t_min"] for record in metadata), "timeMax": max(record["t_max"] for record in metadata),
               "generators": [1], "predicate": DEFAULT_PREDICATE, "aggregate": True}
    return {"sender": sender, "request": request, "data_hashes": data_hashes,
            "inputs": inputs, "receipts": receipts}


def deploy_contract(sender):
    """Forge 为预置开发账户 sender 本地签名，返回合约地址和部署交易回执。"""
    root = Path(__file__).parents[1] / "contracts"
    # 公开的本地开发账户助记词；文件只保存在项目状态卷，不输出私钥。
    mnemonic = Path("/argus/run/query-deployer.mnemonic")
    mnemonic.write_text("test test test test test test test test test test test junk")
    mnemonic.chmod(0o600)
    output = subprocess.check_output([
        "/argus/run/tools/forge", "create", "--root", str(root), "src/Query.sol:Query",
        "--broadcast", "--mnemonic", str(mnemonic), "--from", sender,
        "--legacy", "--gas-price", "1000000000", "--gas-limit", "1000000",
        "--rpc-url", "http://127.0.0.1:8545", "--json"],
        text=True, env={**os.environ, "RAYON_NUM_THREADS": "2"}, timeout=90)
    deployment = json.loads(output)
    receipt = wait_receipts([deployment["transactionHash"]])[0]
    assert receipt["contractAddress"].lower() == deployment["deployedTo"].lower()
    return deployment["deployedTo"], receipt


def setup_demo():
    """导入样本、部署合约并保存查询参数；返回本次部署的全部元数据。"""
    sample = submit_sample()
    contract, deployment = deploy_contract(sample["sender"])
    sample.update(contract=contract, deployment=deployment)
    example = {"contract": contract, "anchor": deployment["blockHash"], "request": sample["request"]}
    Path("/argus/run/query-example.json").write_text(json.dumps(example, ensure_ascii=False, indent=2))
    entry_example = json.loads(json.dumps(example))
    entry_example["request"]["entryPredicate"] = {"op": "keyword", "args": "error"}
    Path("/argus/run/entry-query-example.json").write_text(json.dumps(entry_example, indent=2))
    return sample


def main():
    """生成可直接交给 client.query 使用的容器内查询示例。"""
    sample = setup_demo()
    print(json.dumps({"contract": sample["contract"], "anchor": sample["deployment"]["blockHash"],
                      "request": sample["request"]}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
