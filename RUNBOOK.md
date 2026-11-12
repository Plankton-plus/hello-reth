# Argus BLAS 运行说明

本工程使用 Reth v2.10.0（`f5192ac72d92bc1e656a6939796421af76544af4`）。本轮在 Ubuntu 26.04 的 Docker 中使用 Rust 1.96.1、Forge/Cast 1.5.0、solc 0.8.30 验证。下列命令在 Ubuntu Bash 输入；依赖、编译、数据生成与执行都在容器内完成。

当前方案为 `src/schemes/blas/`，编号 25、版本 1，BF 已移除。日志业务结构沿用上游 BLAS 的 Entry、LogFile、Transaction；Ethereum 签名交易是外层信封。原生完整头和体仍由现有 Reth 框架保存。

## 构建与工具

首次构建镜像、创建编译容器。已存在容器时使用 `docker start argus-blas-v1-build`，不要重复创建。

```bash
cd /home/fangua/argus/reth-node
docker build -t argus/reth-dev:0 .
docker run -d --name argus-blas-v1-build --cpus 2 --memory 3g --memory-swap 3g \
  -v "$PWD:/argus/project" -v argus-reth-cargo:/usr/local/cargo \
  -v argus-reth-target:/argus/target -v argus-blas-v1-run:/argus/run \
  -v argus-shared:/argus/shared argus/reth-dev:0 sleep infinity
docker exec argus-blas-v1-build cargo build --locked --bins
docker exec argus-blas-v1-build cargo test --locked --offline --lib
```

首次新状态卷安装工具，然后编译合约。工具已存在时直接执行 Forge 构建即可。小内存机器重新编译节点前先停止运行节点。

```bash
docker exec argus-blas-v1-build sh /argus/project/install_query_tools.sh
docker exec -e RAYON_NUM_THREADS=2 argus-blas-v1-build \
  /argus/run/tools/forge build --root /argus/project/contracts
```

需要检查 Rust 格式时，在编译容器内安装 rustfmt 后执行。安装仅影响容器工具链。

```bash
docker exec argus-blas-v1-build rustup component add rustfmt
docker exec argus-blas-v1-build cargo fmt --check
```

## 生成共用小样本

生成器读取既有 `/argus/shared/records.jsonl` 的前 24 条记录，转换为六份 BLAS 文件，每份四条。时间戳来自共用记录，样本特意包含相同时间戳和非时间顺序条目。文件公共关键词独立指定，条目关键词与原文由样本转换。Rust 工具实际构建文件 ADS，生成真实的三个认证根。

```bash
docker exec argus-blas-v1-build python3 -B -m scripts.data
docker exec argus-blas-v1-build cat /argus/shared/blas/manifest.json
```

生成的元数据位于 `/argus/shared/blas/metadata.jsonl`，原始文件输入位于 `inputs.jsonl`，完整文件及 ADS 位于 `files/<OMT根>.json`。这些内容保存在 `argus-shared`；运行节点只读挂载。原有 `framework/` 样本保留，本轮不再读取旧元数据格式。

链上 JSON 字段为 `lid`、`t_min`、`t_max`、`tx_public_keys`、`entry_keywords`、`omt_root`、`iimt_root`、`rimbt_root`。生成器为 1～4096 的整数，时间为 u64 秒，区间按闭区间相交，关键词区分大小写并去除两侧空白。`IndexedRecord` 另外绑定真实交易位置和交易哈希，位图编号不直接充当 Ethereum 交易位置。

## 启动节点、导入与查询

BLAS 创世头和 BF 不同，必须使用新状态卷。本轮为 `argus-blas-v1-run`，不复用旧 BF 数据库。首次创建运行容器：

```bash
docker run -d --name argus-blas-v1-node --cpus 2 --memory 3g --memory-swap 3g --stop-timeout 60 \
  -p 127.0.0.1:8550:8545 -v "$PWD:/argus/project:ro" \
  -v argus-reth-target:/argus/target:ro -v argus-blas-v1-run:/argus/run \
  -v argus-shared:/argus/shared:ro -e XDG_CACHE_HOME=/argus/run/cache \
  argus/reth-dev:0 sh /argus/project/run_body_node.sh
docker exec argus-blas-v1-node tail -n 15 /argus/run/node.log
```

已有容器使用 `docker start argus-blas-v1-node`。看到 RPC server started 后执行示例；demo 会提交三批数据及两条排除交易，等待完整四块段，再部署合约。再次执行 demo 会新增一批交易和合约。

```bash
docker exec argus-blas-v1-node python3 -B -m scripts.demo
docker exec argus-blas-v1-node cat /argus/run/query-example.json
docker exec argus-blas-v1-node python3 -B -m client.query /argus/run/query-example.json
docker exec argus-blas-v1-node python3 -B -m client.query /argus/run/entry-query-example.json
```

普通示例查询生成器 1，文件满足 FTP、NOT Admin，以及密码或用户名错误谓词。客户端先调用真实合约，验证链上 VO，再输出两份已认证元数据。条目示例进一步查询 `error`，模拟 SP 工具生成文件 VO，再由独立验证命令检查，共得到四条日志。

ABI 是 `query((bytes32,uint64,uint64,uint64,uint32[],bytes,bool))`，依次传入结束块哈希、起始高度、日志起止时间、生成器数组、文件谓词 JSON 和是否启用聚合。返回 `(QueryMatch[], bytes proof)` 组成的结构体。`entryPredicate` 由客户端传给链下文件查询，不放入节点共识执行；预编译不会读取本地日志文件。

修改参数文件中的 `request.aggregate` 为 false 可走普通路径，为 true 可利用完整窗口。两条路径返回同样的匹配交易，证明格式由客户端独立验证。固定快照 `anchor` 必须晚于结束块。正常查询使用 eth_call，不提交查询交易。

## 单独运行文件工具

数据生成器已在状态卷保存一套文件输入、元数据和条目查询参数。以下命令分别展示构建、SP 查询、客户端验证。最后一条只读取元数据、查询和 VO，不读取原始文件或完整 ADS。

```bash
docker exec argus-blas-v1-build /argus/target/debug/blas-file build \
  /argus/run/file-input.json /argus/run/file-example.json
docker exec argus-blas-v1-build /argus/target/debug/blas-file query \
  /argus/run/file-example.json /argus/run/file-query.json /argus/run/file-vo.json
docker exec argus-blas-v1-build /argus/target/debug/blas-file verify \
  /argus/run/file-metadata.json /argus/run/file-query.json /argus/run/file-vo.json
```

`blas-file verify-chain INPUT` 是链上 VO 的独立验证入口。其 JSON 输入为 `headers`、公共 `query`、`proof`、`matches`，只返回验证通过的业务记录。`client.query` 已负责生成这一输入；通常不需要手工组装。

## 验收与重启恢复

create 会新增小样本并保存验收快照。body 验收核对完整主体、真实交易位置，并检查五类索引伪造；query 验收比较普通和聚合路径、独立交易扫描与 VO 验证，另检查非法请求、真实查询交易及条目证明。

```bash
docker exec argus-blas-v1-node python3 -B -m tests.block_body create
docker exec argus-blas-v1-node python3 -B -m tests.query create
docker restart argus-blas-v1-node
docker exec argus-blas-v1-node tail -n 15 /argus/run/node.log
```

等待 RPC 恢复后，verify 使用保存的历史快照，不重新导入或补建索引。要求完整体保持一致，原始 ABI 结果逐字节相同，并再次通过独立扫描和 VO 验证。

```bash
docker exec argus-blas-v1-node python3 -B -m tests.block_body verify
docker exec argus-blas-v1-node python3 -B -m tests.query verify
docker exec argus-blas-v1-node cat /argus/run/body-result.json
docker exec argus-blas-v1-node cat /argus/run/query-result.json
```

结果文件含完整 ABI 和 VO，终端输出较长。`argus-blas-v1-run` 保存数据库、工具、合约、示例和结果；`argus-shared` 保存共用样本与文件 ADS；编译缓存位于 `argus-reth-cargo`、`argus-reth-target`。如需宿主机持久化，可将本项目 `run/` 绑定到 `/argus/run`，替换上述状态卷挂载；必须从全新目录开始。不要删除既有状态卷来切换方案。

```bash
docker stop argus-blas-v1-node
docker start argus-blas-v1-node
docker stop argus-blas-v1-build
docker start argus-blas-v1-build
```

宿主机 RPC 为 `http://127.0.0.1:8550`；容器内为 8545。8551 的 JWT 实验 Engine RPC 仅由容器内验收调用。

## 复现边界

本轮是 BLAS 分层查询的 Rust 小样本移植：文件 OMT/IIMT/时间索引、块内 OMT/IIMT/双时间位图、窗口生成器/关键词/时间索引和共享关键词下层树均已接入。范围树为批量打包的静态四叉有序树，分隔键由子树最小叶键推导；不提供增量分裂、合并。文件时间叶保存 timestamp→sn，重复时间戳合并存储全部 sn。

上游实际使用 BLAKE2s，本轮沿用 BLAKE2s-256，但采用 RLP、域分离和明确长度承诺，不能读取 C++ Boost 序列化文件或直接比较旧根。VO 使用 JSON，OMT 使用逐对象路径，尚未合并为上游的共享多对象路径；目前 VO 字节数不能作为论文格式下的性能复现结果。

窗口固定四块，参数进入创世头；文件最多 2048 条目，单块最多 256 份元数据。框架仍限制单次查询最多 32 块、位于调用快照之前最近 256 个祖先块内，单块索引与输出各 64 KiB、结果最多 128 条。未开展 1024 块窗口、LogHub 全量压力、多节点传播或标准共识客户端互操作实验。

客户端沿父哈希重算完整头，并以调用方选定的结束块哈希作为信任锚；本原型不实现获取可信检查点或 PoS 轻客户端。普通及聚合查询都验证缺失键、范围边界、候选位图和结果完整性；错误不会转换为空结果，也没有扫描降级或自动补建。
