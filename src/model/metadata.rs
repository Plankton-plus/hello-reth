//! BLAS Transaction 的链上元数据；Ethereum 签名交易作为外层信封。
use alloy_primitives::B256;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
#[serde(deny_unknown_fields)]
pub struct LogMetadata {
    pub lid: u32, //标志这份文件属于哪个生成器
    pub t_min: u64,  //文件内所有条目时间戳的最小值
    pub t_max: u64,//最大值
    pub tx_public_keys: Vec<String>, //文件级关键字
    pub entry_keywords: Vec<String>,//整个文件所有条目的词表并集
    pub omt_root: B256,//把链下文件内容承诺到链上密码学，承诺全部条目本体。查询命中某条日志后，SO给出的OMT路径
    pub iimt_root: B256,//倒排关键词索引
    pub rimbt_root: B256,//时间范围索引，并额外绑定了条目数量
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
pub struct IndexedRecord {
    pub transaction_index: u32,
    pub transaction_hash: B256,
    pub metadata: LogMetadata,
}

/// 规范化 words；limit 限制词表长度，保持关键词区分大小写。
pub fn normalize_words(words: &mut Vec<String>, limit: usize) -> Result<(), String> {
    if words.len() > limit {
        return Err("关键词数量超过协议限制".into());
    }
    for word in words.iter_mut() {
        *word = word.trim().to_owned();
        if word.is_empty() || word.len() > 64 {
            return Err("关键词须为 1～64 字节".into());
        }
    }
    words.sort_unstable();
    words.dedup();
    Ok(())
}

impl LogMetadata {
    /// 解析 line；三个根独立命名并参与 OMT 承诺。
    pub fn parse(line: &str) -> Result<Self, String> {
        let mut value: Self =
            serde_json::from_str(line).map_err(|e| format!("BLAS 元数据 JSON 非法：{e}"))?;
        if value.lid == 0 || value.lid > 4096 || value.t_min > value.t_max {
            return Err("生成器或日志时间范围非法".into());
        }
        if [value.omt_root, value.iimt_root, value.rimbt_root].contains(&B256::ZERO) {
            return Err("文件索引认证根不能为零".into());
        }
        normalize_words(&mut value.tx_public_keys, 256)?;
        normalize_words(&mut value.entry_keywords, 4096)?;
        Ok(value)
    }
}
