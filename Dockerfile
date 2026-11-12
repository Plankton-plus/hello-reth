# Argus：Reth 原型的 Rust 编译与运行环境。
FROM rust:1.96-bookworm

RUN sed -i 's/deb.debian.org/mirrors.tuna.tsinghua.edu.cn/g' /etc/apt/sources.list.d/debian.sources && \
    apt-get update && apt-get install -y --no-install-recommends \
    build-essential clang lld libclang-dev pkg-config libssl-dev \
    cmake protobuf-compiler git curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_BUILD_JOBS=1 \
    CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_TARGET_DIR=/argus/target \
    RUSTFLAGS="-C link-arg=-fuse-ld=lld"

WORKDIR /argus/project
CMD ["bash"]
