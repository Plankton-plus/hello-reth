//! 公共查询模型：生成器集合、日志时间闭区间和关键词布尔谓词。
//! 数据流：合约传入QueryRequest
//! query/execure.rs decode_query()会调用LogQuery::new(...)得到LogQuery对象
//! scheme.rs方案内部会调用query.matches精准判断每条记录
//! 返回命中的记录

use super::{metadata::LogMetadata, predicate::Predicate};

#[derive(Clone, Debug)]
pub struct LogQuery { //note:这里查询模型可能遗漏，可能会有范围
    pub generators: Vec<String>, //只看哪些生成器产生的日志
    pub time: (u64, u64),//限制时间范围
    pub predicate: Predicate, //限定关键词逻辑
}

impl LogQuery {
    /// 校验 generators、time，并解码 predicate；空生成器集合表示全部生成器。
    pub fn new(
        mut generators: Vec<String>,
        time: (u64, u64),
        predicate: &[u8],
    ) -> Result<Self, String> {
        if time.0 > time.1 || generators.len() > 16 {
            return Err("查询时间倒置或生成器超过 16 个".into());
        }
        for generator in &mut generators {
            *generator = generator.trim().to_owned();
            if generator.is_empty() || generator.len() > 64 {
                return Err("生成器标识必须为 1～64 字节".into());
            }
        }
        generators.sort_unstable();
        generators.dedup();
        Ok(Self {
            generators,
            time,
            predicate: Predicate::decode(predicate)?,
        })
    }

    /// 精确判断 metadata；文件时间区间与查询闭区间相交即满足时间条件。 查看一条日志元数据是否满足这个查询
    pub fn matches(&self, metadata: &LogMetadata) -> bool {
        metadata.ts_min <= self.time.1
            && metadata.ts_max >= self.time.0
            && (self.generators.is_empty() || self.generators.contains(&metadata.lid))
            && self.predicate.matches(&metadata.fw)
    }
}
