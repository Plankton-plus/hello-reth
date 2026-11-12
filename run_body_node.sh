#!/bin/sh
# 完整区块体实验节点；数据库、索引表、密钥和日志均位于独立状态卷。
set -eu

exec /argus/target/debug/indexed-node node \
  --dev --dev.block-time 5s --datadir /argus/run/data \
  --http --http.addr 0.0.0.0 --http.api eth,net,web3 \
  >> /argus/run/node.log 2>&1
