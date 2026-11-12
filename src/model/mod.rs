//! 各方案共用的日志元数据和查询语义。

pub mod metadata; //日志文件元数据
pub mod predicate;//关键词 AND、OR、NOT
pub mod query; //生成器、时间范围、关键词查询
pub mod record; //从交易中提取原始元数据
