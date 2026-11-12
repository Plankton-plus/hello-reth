//! 经 JWT 认证的实验 Engine RPC，提交完整 payload 并读回原生扩展区块。rpc就是对外的门面，将前面辛苦实现的索引payload、索引校验
//! 索引存储，统一包装成三个可通过HTTP/JWT调用的接口。

use crate::{
    node::block::IndexedPrimitives,
    node::chainspec::IndexedChainSpec,
    node::payload::{IndexedEngineTypes, IndexedExecutionData},
    node::validation::{IndexedPayloadValidator, IndexedValidatorBuilder},
};
use alloy_primitives::B256;
use jsonrpsee::{RpcModule, types::ErrorObjectOwned};
use reth_ethereum::{
    node::{
        api::{AddOnsContext, FullNodeComponents, NodeTypes, PayloadTypes, PayloadValidator},
        builder::rpc::{EngineApiBuilder, PayloadValidatorBuilder},
    },
    provider::BlockReader,
    rpc::api::IntoEngineApiRpcModule,
};

pub struct IndexedEngineApi(RpcModule<()>);
impl IntoEngineApiRpcModule for IndexedEngineApi {
    /// 返回实验链专用认证 RPC，不安装会丢失索引的标准 newPayload 接口。
    fn into_rpc_module(self) -> RpcModule<()> {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexedEngineApiBuilder;
impl<N> EngineApiBuilder<N> for IndexedEngineApiBuilder
where
    N: FullNodeComponents<
        Types: NodeTypes<
            ChainSpec = IndexedChainSpec,
            Primitives = IndexedPrimitives,
            Payload = IndexedEngineTypes,
        >,
    >,
{
    type EngineApi = IndexedEngineApi;
    /// ctx 提供引擎句柄、读取器和真实校验器，所有请求均进入原引擎。
    async fn build_engine_api(self, ctx: &AddOnsContext<'_, N>) -> eyre::Result<IndexedEngineApi> {
        let validator: IndexedPayloadValidator = IndexedValidatorBuilder.build(ctx).await?;
        let context = (
            ctx.beacon_engine_handle.clone(),
            ctx.node.provider().clone(),
            validator,
        );
        let mut module = RpcModule::new(context);
        module.register_async_method("argus_newPayload", |params, ctx, _| async move {
            let payload: IndexedExecutionData = params.one()?;
            // 缓存命中前核对索引承诺，避免改动主体后绕过接收检查。
            if let Err(error) = ctx.2.convert_payload_to_block(payload.clone()) {
                return Ok(serde_json::json!({"status":"INVALID", "validationError":error.to_string(), "latestValidHash":null}));
            }
            let status = ctx.0.new_payload(payload).await.map_err(|e| ErrorObjectOwned::owned(-32000, e.to_string(), None::<()>))?;
            Ok::<_, ErrorObjectOwned>(serde_json::to_value(status).expect("状态可序列化"))
        })?;
        module.register_method("argus_getBlock", |params, ctx, _| {
            let hash: B256 = params.one()?;
            ctx.1
                .block_by_hash(hash)
                .map_err(|e| ErrorObjectOwned::owned(-32000, e.to_string(), None::<()>))
        })?;
        module.register_method("argus_exportPayload", |params, ctx, _| {
            let hash: B256 = params.one()?;
            let block = ctx
                .1
                .block_by_hash(hash)
                .map_err(|e| ErrorObjectOwned::owned(-32000, e.to_string(), None::<()>))?;
            Ok::<_, ErrorObjectOwned>(block.map(|block| {
                IndexedEngineTypes::block_to_payload(
                    reth_ethereum::primitives::Block::seal_slow(block),
                    None,
                )
            }))
        })?;
        Ok(IndexedEngineApi(module.remove_context()))
    }
}
