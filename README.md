# Argus：BLAS 分层可验证日志查询

Reth 构建、校验并保存完整索引，Solidity 合约通过预编译查询，客户端独立验证文件元数据和日志条目。当前方案编号为 25、版本为 3，四块组成一段。

节点基于 [Reth v2.10.0](https://github.com/paradigmxyz/reth/tree/f5192ac72d92bc1e656a6939796421af76544af4)；算法参考 [BLAS25](https://github.com/BLAS25/BLAS25/tree/6cd064932d9dfd9bdc54c660cd3da81f6b766d68)，采用 Rust 独立实现。已使用 Rust 1.96.1、Forge/Cast 1.5.0 和 solc 0.8.30 验证。

索引位于 `src/schemes/blas/`：`common/` 存放位图、Merkle 树和过滤运算，`index/` 分为条目、块内文件记录、分段和生成器聚合四部分，`query.rs` 负责检索及 VO 生成，`proof.rs` 负责独立验证。查询链路为客户端 → Query.sol → 预编译 0xa901 → BLAS → 客户端验证 → 链下条目查询与验证。

## 容器构建

以下命令在 Ubuntu Bash 执行，编译和程序运行均在容器内完成。首次创建容器使用下列命令；容器已经存在时改用 `docker start`，保留已有状态卷。

```bash
cd /home/fangua/argus/reth-node
docker build -t argus/reth-dev:0 .
docker run -d --name argus-blas-v3-build --cpus 2 --memory 3g --memory-swap 3g \
  -v "$PWD:/argus/project" -v argus-reth-cargo:/usr/local/cargo \
  -v argus-reth-target:/argus/target -v argus-blas-v3-run:/argus/run \
  -v argus-shared:/argus/shared:ro argus/reth-dev:0 sleep infinity
docker exec argus-blas-v3-build cargo build --locked --bins
docker exec argus-blas-v3-build cargo test --locked --offline --lib
docker exec argus-blas-v3-build sh /argus/project/install_query_tools.sh
docker exec -e RAYON_NUM_THREADS=2 argus-blas-v3-build \
  /argus/run/tools/forge build --root /argus/project/contracts
```

首次构建需要下载依赖；缓存完整后可为 `cargo build` 添加 `--offline`。工具安装只需执行一次。小内存机器重新编译前可先停止节点，编译完成后再启动。

## 数据准备

已有公共样本时直接转换；首次初始化公共样本执行 `bash ../scripts/seed-shared.sh`。转换器从 `records.jsonl` 取 24 条记录，生成六份文件及实际认证根。仅生成器写入共享卷，节点和编译容器只读挂载。

```bash
docker run --rm -v "$PWD:/argus/project:ro" -v argus-reth-target:/argus/target:ro \
  -v argus-blas-v3-run:/argus/run -v argus-shared:/argus/shared \
  argus/reth-dev:0 python3 -B -m scripts.data
docker exec argus-blas-v3-build cat /argus/shared/blas/manifest.json
```

文件元数据位于 `/argus/shared/blas/metadata.jsonl`，完整文件及索引位于 `files/<OMT根>.json`。生成器同时在状态卷保存 `file-input.json`、`file-metadata.json` 和 `file-query.json`，供单文件命令使用。

真实日志接入前，需要先转换为 FileInput：lid 表示生成器，tx_public_keywords 表示文件关键字，entries 中每条记录包含 sn、timestamp、keywords、log_value。sn 从零连续编号，时间使用整数秒，允许重复和乱序。当前 `scripts.data` 和 `scripts.demo` 仍是固定小样本工具，尚未实现 LogHub 原始文本的批量导入。

## 启动节点

首次创建运行容器；更换方案或索引版本时使用新状态卷。宿主机 RPC 为 http://127.0.0.1:8553，容器内端口为 8545。

```bash
docker run -d --name argus-blas-v3-node --cpus 2 --memory 3g --memory-swap 3g --stop-timeout 60 \
  -p 127.0.0.1:8553:8545 -v "$PWD:/argus/project:ro" \
  -v argus-reth-target:/argus/target:ro -v argus-blas-v3-run:/argus/run \
  -v argus-shared:/argus/shared:ro -e XDG_CACHE_HOME=/argus/run/cache \
  argus/reth-dev:0 sh /argus/project/run_body_node.sh
docker exec argus-blas-v3-node tail -n 15 /argus/run/node.log
```

## 部署与查询

看到 `RPC server started` 后，首次运行 demo 导入样本并部署查询合约。重复执行会新增交易和合约；只查询已有数据时直接使用保存的参数文件。

```bash
docker exec argus-blas-v3-node python3 -B -m scripts.demo
docker exec argus-blas-v3-node cat /argus/run/query-example.json
docker exec argus-blas-v3-node python3 -B -m client.query /argus/run/query-example.json
docker exec argus-blas-v3-node python3 -B -m client.query /argus/run/entry-query-example.json
```

query-example.json 保存 contract、anchor 和 request。request 包含 endBlockHash、startBlock、timeMin、timeMax、generators、predicate、aggregate；条目查询额外使用 entryPredicate。文件谓词进入链上查询，条目谓词只用于链下文件。时间采用闭区间，文件时间按区间相交筛选，关键字区分大小写。

将 request.aggregate 设为 false 使用普通查询，设为 true 启用块头时间剪枝及完整段聚合。anchor 是调用快照，必须晚于结束块。查询通过 eth_call 执行；客户端验证链上 VO，条目查询继续验证文件证明。成功输出 `verified: true`；当前默认样本返回两份文件元数据、四条匹配日志。

## 单文件构建、查询和验证

准备 /argus/run/file-input.json 和 file-query.json 后，可单独运行下列命令。build 输出的元数据作为验证依据；verify 只读取元数据、查询和 VO。

```bash
docker exec argus-blas-v3-build sh -c '/argus/target/debug/blas-file build /argus/run/file-input.json /argus/run/file-example.json > /argus/run/file-metadata.json'
docker exec argus-blas-v3-build /argus/target/debug/blas-file query \
  /argus/run/file-example.json /argus/run/file-query.json /argus/run/file-vo.json
docker exec argus-blas-v3-build /argus/target/debug/blas-file verify \
  /argus/run/file-metadata.json /argus/run/file-query.json /argus/run/file-vo.json
```

## 启停与持久化

已有容器直接启停，日志位于状态卷的 `/argus/run/node.log`。

```bash
docker stop argus-blas-v3-node
docker start argus-blas-v3-node
docker stop argus-blas-v3-build
docker start argus-blas-v3-build
docker exec argus-blas-v3-node tail -n 15 /argus/run/node.log
```

argus-blas-v3-run 保存数据库、工具和查询参数；argus-shared 保存公共样本及文件索引；argus-reth-cargo 和 argus-reth-target 保存编译缓存。需要宿主机文件时，可在创建容器前将状态卷改为本项目 run/ 目录的绑定挂载。旧版本数据库保持独立，不删除已有卷。

## 当前限制

每个文件最多 2048 条日志，每块最多 256 份元数据，段大小固定四块。单次查询最多 32 块，范围位于调用快照之前最近 256 个祖先块内；单块索引和查询输出各限 64 KiB，链上结果最多 128 份文件元数据。真实数据运行应先控制在这些限制内。

范围索引采用静态有序 Merkle 树，VO 为 JSON。完整段且指定生成器时使用共享对象证明；关键字下层树仍逐块提供路径，其余对象查询使用普通路径。哈希使用 BLAKE2s-256，编码采用 RLP，与上游 C++ 序列化和认证根不直接兼容。客户端需要调用方提供可信结束块哈希，当前原型不包含获取共识检查点的轻客户端。
