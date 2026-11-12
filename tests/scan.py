"""独立扫描标准交易 RPC，直接判断元数据，不读取任何索引。"""

import json

from client.rpc import rpc
from client.transactions import DATA_ADDRESS


def evaluate(expression, words):
    """按 expression 的显式括号结构，在 words 集合上独立求布尔值。"""
    operation, arguments = expression["op"], expression.get("args")
    if operation == "all":
        return True
    if operation == "keyword":
        return arguments.strip() in words
    if operation == "and":
        return all(evaluate(child, words) for child in arguments)
    if operation == "or":
        return any(evaluate(child, words) for child in arguments)
    if operation == "not":
        return not evaluate(arguments, words)
    raise AssertionError("扫描器遇到未知操作")


def scan_transactions(request):
    """沿固定父哈希扫描 request 范围，比较生成器、日志区间和布尔谓词。"""
    blocks, block_hash = [], request["endBlockHash"]
    while True:
        block = rpc("eth_getBlockByHash", [block_hash, True])
        assert block, "扫描所需区块不存在"
        blocks.append(block)
        if int(block["number"], 16) == request["startBlock"]:
            break
        block_hash = block["parentHash"]
    found = []
    generators = set(request["generators"])
    for block in reversed(blocks):
        for position, tx in enumerate(block["transactions"]):
            payload = bytes.fromhex(tx["input"][2:])
            if tx["to"] != DATA_ADDRESS or not payload.startswith(b"ARGUS2\0"):
                continue
            metadata = json.loads(payload[7:])
            words = set(metadata["tx_public_keys"])
            if ((not generators or metadata["lid"] in generators)
                    and metadata["t_min"] <= request["timeMax"] and metadata["t_max"] >= request["timeMin"]
                    and evaluate(request["predicate"], words)):
                found.append({"blockHash": block["hash"], "transactionIndex": position,
                              "transactionHash": tx["hash"],
                              "rawTransaction": rpc("eth_getRawTransactionByHash", [tx["hash"]])})
    return found
