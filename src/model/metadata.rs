//! note:这个本该设计成一个日志文件的元数据，而不是一个日志元数据

use alloy_primitives::{B256, Bytes};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogMetadata {
    pub lid: String, //生成器标志
    pub fid: String, //文件标志
    pub ts_min: u64, //时间区间的下届
    pub ts_max: u64, //时间区间的上届
    pub fw: Vec<String>, //文件关键词，用于布尔检索
    pub bm_ew: Bytes, //二进制字段，可能表示布隆/位图编码1
    pub rh: Vec<B256>, //引用哈希
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedRecord {
    pub transaction_index: u32, //这条记录对应原始交易在区块中的下标
    pub metadata: LogMetadata,
}

//note:line是一段JSON文本
impl LogMetadata {
    /// 解析 line 中的元数据；统一规定 lid 为生成器标识，fid 为文件标识。
    pub fn parse(line: &str) -> Result<Self, String> {
        let mut value: Self =
            serde_json::from_str(line).map_err(|e| format!("元数据 JSON 非法：{e}"))?;
        if value.lid.trim().is_empty()
            || value.fid.trim().is_empty()
            || value.lid.len() > 64
            || value.fid.len() > 64
        {
            return Err("lid 和 fid 必须为 1～64 字节的标识".into());
        }
        if value.ts_min > value.ts_max || value.fw.len() > 256 {
            return Err("日志时间区间倒置或文件关键词超过 256 个".into());
        }
        value.lid = value.lid.trim().to_owned();
        value.fid = value.fid.trim().to_owned();
        for word in &mut value.fw {
            *word = word.trim().to_owned();
            if word.is_empty() || word.len() > 64 {
                return Err("文件关键词必须为 1～64 字节".into());
            }
        }
        value.fw.sort_unstable();
        value.fw.dedup();
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 日志时间与载荷一致，倒置区间和未知字段不能进入共同数据模型。
    #[test]
    fn validates_metadata_protocol() {
        let line = crate::test_support::line();
        let metadata = LogMetadata::parse(line).unwrap();
        assert_eq!((metadata.ts_min, metadata.ts_max), (100, 110));
        assert!(LogMetadata::parse(&line.replace("\"ts_min\":100", "\"ts_min\":111")).is_err());
        assert!(LogMetadata::parse(&line.replace("\"lid\":\"LG_1\"", "\"lid\":\" \"")).is_err());
        assert!(LogMetadata::parse(&line.replace("\"rh\":[]", "\"rh\":[],\"extra\":1")).is_err());
    }
}
