//! BLAS 业务查询；文件和条目使用相互独立的关键词谓词。
use super::{metadata::LogMetadata, predicate::Predicate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogQuery {
    pub generators: Vec<u32>,
    pub time: (u64, u64),
    pub predicate: Predicate,
    #[serde(default)]
    pub entry_predicate: Option<Predicate>,
    #[serde(default)]
    pub aggregate: bool,
}

impl LogQuery {
    /// 校验从文件读取的业务查询；复用合约入口的限制，不接受未规范化谓词。
    pub fn normalize(self) -> Result<Self, String> {
        let mut value = Self::new(
            self.generators,
            self.time,
            &serde_json::to_vec(&self.predicate).map_err(|e| e.to_string())?,
        )?;
        value.entry_predicate = self
            .entry_predicate
            .map(|p| Predicate::decode(&serde_json::to_vec(&p).map_err(|e| e.to_string())?))
            .transpose()?;
        value.aggregate = self.aggregate;
        Ok(value)
    }
    /// 创建文件级查询；空 generators 表示全部，time 使用闭区间。
    pub fn new(
        mut generators: Vec<u32>,
        time: (u64, u64),
        predicate: &[u8],
    ) -> Result<Self, String> {
        if time.0 > time.1
            || generators.len() > 16
            || generators.iter().any(|id| *id == 0 || *id > 4096)
        {
            return Err("查询生成器或时间范围非法".into());
        }
        generators.sort_unstable();
        generators.dedup();
        Ok(Self {
            generators,
            time,
            predicate: Predicate::decode(predicate)?,
            entry_predicate: None,
            aggregate: true,
        })
    }
    /// 精确判断 metadata；文件时间区间与查询闭区间相交。
    pub fn matches(&self, metadata: &LogMetadata) -> bool {
        metadata.t_min <= self.time.1
            && metadata.t_max >= self.time.0
            && (self.generators.is_empty() || self.generators.contains(&metadata.lid))
            && self.predicate.matches(&metadata.tx_public_keys)
    }
}
