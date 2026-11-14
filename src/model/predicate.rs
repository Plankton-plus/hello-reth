//! 关键词布尔表达式；NOT 针对单条元数据的关键词集合解释。
//! 合约传入 predicate JSON 字节，解析为带显式括号的表达式树。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "args",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum Predicate {
    //表达式定义，递归枚举
    All,
    Keyword(String),
    And(Vec<Self>),
    Or(Vec<Self>),
    Not(Box<Self>),
}

impl Predicate {
    /// 从合约calldata里传过来的就是原始字节，反序列化
    pub fn decode(encoded: &[u8]) -> Result<Self, String> {
        let mut value: Self =
            serde_json::from_slice(encoded).map_err(|e| format!("布尔表达式非法：{e}"))?;
        value.normalize(0, &mut 0)?;
        Ok(value)
    }

    /// 规范化当前节点；depth 和 count 限制递归和工作量。
    fn normalize(&mut self, depth: usize, count: &mut usize) -> Result<(), String> {
        *count += 1;
        if depth > 16 || *count > 64 {
            return Err("布尔表达式最多 64 个节点、16 层".into());
        }
        match self {
            Self::All => (),
            Self::Keyword(word) => {
                *word = word.trim().to_owned();
                if word.is_empty() || word.len() > 64 {
                    return Err("查询关键词必须为 1～64 字节".into());
                }
            }
            Self::And(children) | Self::Or(children) => {
                if children.is_empty() {
                    return Err("AND 和 OR 至少需要一个操作数".into());
                }
                for child in children {
                    child.normalize(depth + 1, count)?;
                }
            }
            Self::Not(child) => child.normalize(depth + 1, count)?,
        }
        Ok(())
    }

    /// 对 words 做精确判断，作为所有方案共同遵守的布尔语义。
    pub fn matches(&self, words: &[String]) -> bool {
        match self {
            Self::All => true,
            Self::Keyword(word) => words.contains(word),
            Self::And(children) => children.iter().all(|child| child.matches(words)),
            Self::Or(children) => children.iter().any(|child| child.matches(words)),
            Self::Not(child) => !child.matches(words),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定 NOT 与 OR 的真值，并拒绝未知操作和空逻辑节点。
    #[test]
    fn validates_boolean_semantics() {
        let predicate =
            Predicate::decode(br#"{"op":"not","args":{"op":"keyword","args":"Admin"}}"#).unwrap();
        assert!(predicate.matches(&["FTP".into()]));
        assert!(!predicate.matches(&["Admin".into()]));
        assert!(Predicate::decode(br#"{"op":"and","args":[]}"#).is_err());
        assert!(Predicate::decode(br#"{"op":"xor","args":[]}"#).is_err());
        assert!(Predicate::decode(br#"{"op":"keyword","args":" "}"#).is_err());
    }
}
