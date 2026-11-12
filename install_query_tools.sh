#!/bin/sh
# 仅在容器内运行：固定版本和 SHA256，工具持久化到本项目状态卷。
set -eu
mkdir -p /argus/run/tools
cd /argus/run/tools
curl -fLsS --connect-timeout 15 --max-time 900 \
    https://github.com/foundry-rs/foundry/releases/download/v1.5.0/foundry_v1.5.0_linux_amd64.tar.gz \
    -o foundry.tar.gz
echo '5cd98f9092bcc28be087939491f786b2bf3ed55e492996a409e29519b8ab4dc8  foundry.tar.gz' | sha256sum -c -
tar -xzf foundry.tar.gz forge cast
curl -fLsS --connect-timeout 15 --max-time 900 \
    https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.8.30+commit.73712a01 \
    -o solc
echo 'f3e987dc6ecebd4bd350c48edcbc320b46cf9e3109bd3fc3d88f1acaf4c428f7  solc' | sha256sum -c -
chmod +x forge cast solc
./forge --version
./cast --version
./solc --version
