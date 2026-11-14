//! IIMT 与双端点范围位图；同一布尔语义用于服务端和独立验证器。
//! 被块级索引BlockIndex持有
//! 字段filters：Filters
//! build 调 Filters::build（从块内 records 抽 keys/times/lids）；
//! roots 用 filters.roots() 拼出 5 个根（OMT + 四过滤树）；
//! 被段级索引持有
//! filters::Filters
//! build输入微keys/times/lids

use super::{
    bitmap::Bitmap,
    ordered::{OrderedIndex, OrderedProof, decode},
};
use crate::model::{predicate::Predicate, query::LogQuery};
use alloy_primitives::{B256, Bytes};
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct Filters {
    pub keywords: OrderedIndex, //词->位图
    pub min_time: OrderedIndex,//min_time
    pub max_time: OrderedIndex,//max_time
    pub generators: OrderedIndex,//lid
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilterProof {
    pub keywords: Vec<OrderedProof>,
    pub min_time: OrderedProof,
    pub max_time: Option<OrderedProof>,
    pub generators: Vec<OrderedProof>,
}

/// 将 u64 编为保持数值顺序的索引键，避免字符串字典序改变范围语义。
pub fn number(value: u64) -> String {
    format!("{value:020}")
}

/// 提取 predicate 涉及的去重关键词，证明顺序与表达式遍历顺序无关。
pub fn words(predicate: &Predicate) -> BTreeSet<String> {
    match predicate {
        Predicate::All => BTreeSet::new(),
        Predicate::Keyword(word) => BTreeSet::from([word.clone()]),
        Predicate::And(children) | Predicate::Or(children) => {
            children.iter().flat_map(words).collect()
        }
        Predicate::Not(child) => words(child),
    }
}

/// 递归执行位图布尔运算；coarse 模式的 NOT 保守放行，避免块级词并集漏报。
pub(crate) fn evaluate(
    predicate: &Predicate,
    values: &BTreeMap<String, Bitmap>,
    len: usize,
    coarse: bool,
) -> Bitmap {
    match predicate {
        Predicate::All => Bitmap::new(len, true),
        Predicate::Keyword(word) => values[word].clone(),
        Predicate::Not(_) if coarse => Bitmap::new(len, true),
        Predicate::Not(child) => evaluate(child, values, len, false).complement(),
        Predicate::And(children) | Predicate::Or(children) => {
            let union = matches!(predicate, Predicate::Or(_));
            children
                .iter()
                .fold(Bitmap::new(len, !union), |acc, child| {
                    acc.combine(&evaluate(child, values, len, coarse), union)
                })
        }
    }
}

impl Filters {
    /// 验证四份有序树的完整内容和哈希缓存。
    pub fn validate(&self) -> Result<(), String> {
        for tree in [
            &self.keywords,
            &self.min_time,
            &self.max_time,
            &self.generators,
        ] {
            tree.validate()?;
        }
        Ok(())
    }
    /// 从 words、times、generators 的同序对象构建索引；允许相同时间戳。
    pub fn build(keys: &[Vec<String>], times: &[(u64, u64)], lids: &[Vec<u32>]) -> Self {
        let len = keys.len();
        assert_eq!(times.len(), len);
        assert_eq!(lids.len(), len);
        let keyword_values = keys
            .iter()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|key| {
                let bitmap = Bitmap::positions(len, (0..len).filter(|i| keys[*i].contains(&key)));
                (key, alloy_rlp::encode(bitmap).into())
            })
            .collect();
        let generator_values = lids
            .iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|lid| {
                (
                    number(lid as u64),
                    alloy_rlp::encode(Bitmap::positions(
                        len,
                        (0..len).filter(|i| lids[*i].contains(&lid)),
                    ))
                    .into(),
                )
            })
            .collect();
        let range = |maximum: bool| -> BTreeMap<String, Bytes> {
            let values: Vec<_> = times
                .iter()
                .map(|time| if maximum { time.1 } else { time.0 })
                .collect();
            values
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|value| {
                    let bitmap = Bitmap::positions(len, (0..len).filter(|i| values[*i] <= value));
                    (number(value), alloy_rlp::encode(bitmap).into())
                })
                .collect()
        };
        Self {
            keywords: OrderedIndex::build(keyword_values, 2),
            min_time: OrderedIndex::build(range(false), 4),
            max_time: OrderedIndex::build(range(true), 4),
            generators: OrderedIndex::build(generator_values, 2),
        }
    }

    /// 认证根的顺序为关键词、开始时间、结束时间、生成器。
    pub fn roots(&self) -> Vec<B256> {
        vec![
            self.keywords.tree.root(),
            self.min_time.tree.root(),
            self.max_time.tree.root(),
            self.generators.tree.root(),
        ]
    }

    /// 为 query 构造过滤证明；include_keywords 为 false 时由段索引认证关键词。
    pub fn prove(&self, query: &LogQuery, include_keywords: bool) -> FilterProof {
        FilterProof {
            keywords: if include_keywords {
                words(&query.predicate)
                    .iter()
                    .map(|key| self.keywords.prove(key))
                    .collect()
            } else {
                Vec::new()
            },
            min_time: self.min_time.prove(&number(query.time.1)),
            max_time: query
                .time
                .0
                .checked_sub(1)
                .map(|value| self.max_time.prove(&number(value))),
            generators: query
                .generators
                .iter()
                .map(|lid| self.generators.prove(&number(*lid as u64)))
                .collect(),
        }
    }
}

impl FilterProof {
    /// 仅由可信 roots、对象数 len 与 query 重算结果位图；不读取索引。
    pub fn verify(
        &self,
        roots: &[B256],
        len: usize,
        query: &LogQuery,
        coarse: bool,
    ) -> Result<Bitmap, String> {
        self.verify_with_keywords(roots, len, query, coarse, None)
    }

    /// 聚合路径可提供已由窗口下层树认证的关键词位图，避免重复块内 IIMT 路径。
    pub fn verify_with_keywords(
        &self,
        roots: &[B256],
        len: usize,
        query: &LogQuery,
        coarse: bool,
        authenticated: Option<&BTreeMap<String, Bitmap>>,
    ) -> Result<Bitmap, String> {
        if roots.len() != 4 || len > 1_000_000 {
            return Err("过滤索引根或对象数非法".into());
        }
        let bitmap = |encoded: Option<Bytes>| -> Result<Bitmap, String> {
            let result = encoded
                .map(|value| decode::<Bitmap>(&value))
                .transpose()?
                .unwrap_or_else(|| Bitmap::new(len, false));
            result.validate(len)?;
            Ok(result)
        };
        let requested: Vec<_> = words(&query.predicate).into_iter().collect();
        if self.generators.len() != query.generators.len() {
            return Err("生成器证明不完整".into());
        }
        let mut values = BTreeMap::new();
        if let Some(authenticated) = authenticated {
            if !self.keywords.is_empty()
                || authenticated.keys().cloned().collect::<Vec<_>>() != requested
            {
                return Err("聚合关键词证明不完整".into());
            }
            for (key, value) in authenticated {
                value.validate(len)?;
                values.insert(key.clone(), value.clone());
            }
        } else {
            if self.keywords.len() != requested.len() {
                return Err("关键词证明不完整".into());
            }
            for (key, proof) in requested.iter().zip(&self.keywords) {
                values.insert(key.clone(), bitmap(proof.exact(roots[0], key)?)?);
            }
        }
        let mut result = evaluate(&query.predicate, &values, len, coarse);
        let lower = self
            .min_time
            .floor(roots[1], &number(query.time.1))?
            .map(|leaf| leaf.value);
        result = result.combine(&bitmap(lower)?, false);
        let excluded = match (query.time.0.checked_sub(1), &self.max_time) {
            (Some(value), Some(proof)) => bitmap(
                proof
                    .floor(roots[2], &number(value))?
                    .map(|leaf| leaf.value),
            )?,
            (None, None) => Bitmap::new(len, false),
            _ => return Err("范围证明缺失或多余".into()),
        };
        result = result.combine(&excluded.complement(), false);
        if !query.generators.is_empty() {
            let mut generators = Bitmap::new(len, false);
            for (lid, proof) in query.generators.iter().zip(&self.generators) {
                generators = generators
                    .combine(&bitmap(proof.exact(roots[3], &number(*lid as u64))?)?, true);
            }
            result = result.combine(&generators, false);
        }
        Ok(result)
    }
}
