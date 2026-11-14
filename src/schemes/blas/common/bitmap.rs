//! 有明确有效长度的位图；补位为零，NOT 不产生不存在的对象。
use alloy_primitives::Bytes;
use alloy_rlp::{RlpDecodable, RlpEncodable};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, RlpEncodable, RlpDecodable,
)]
pub struct Bitmap {
    pub len: u32,    //有效对象数，真实对象数不一定是8的倍数
    pub bits: Bytes, //位存储
}

impl Bitmap {
    /// 创建 len 位的零或一位图；full 决定初值。
    pub fn new(len: usize, full: bool) -> Self {
        let mut bits = vec![if full { 255 } else { 0 }; len.div_ceil(8)]; //向上取整，算出需要多少个字节
        if full && len % 8 != 0 {
            *bits.last_mut().unwrap() &= (1 << (len % 8)) - 1;
        }
        Self {
            len: len as u32,
            bits: bits.into(),
        }
    }
    /// 从逻辑对象位置 positions 构造 len 位位图。
    pub fn positions(len: usize, positions: impl IntoIterator<Item = usize>) -> Self {
        let mut bits = vec![0; len.div_ceil(8)];
        for position in positions {
            assert!(position < len);
            bits[position / 8] |= 1 << (position % 8);
        }
        Self {
            len: len as u32,
            bits: bits.into(),
        }
    }
    /// 返回置位位置，保持对象顺序。
    pub fn indices(&self) -> Vec<usize> {
        (0..self.len as usize)
            .filter(|i| self.bits[i / 8] & (1 << (i % 8)) != 0)
            .collect()
    }
    /// 校验实际字节数和补位；len 为预期对象数。
    pub fn validate(&self, len: usize) -> Result<(), String> {
        if self.len as usize != len || self.bits.len() != len.div_ceil(8) {
            return Err("位图长度不匹配".into());
        }
        if len % 8 != 0 && self.bits.last().is_some_and(|b| *b >> (len % 8) != 0) {
            return Err("位图补位非零".into());
        }
        Ok(())
    }
    /// 将 other 与本位图结合；union 选择 OR 或 AND。
    pub fn combine(&self, other: &Self, union: bool) -> Self {
        assert_eq!(self.len, other.len);
        Self {
            len: self.len,
            bits: self
                .bits
                .iter()
                .zip(other.bits.iter())
                .map(|(a, b)| if union { a | b } else { a & b })
                .collect::<Vec<_>>()
                .into(),
        }
    }
    /// 仅对有效对象取补集。
    pub fn complement(&self) -> Self {
        let universe = Self::new(self.len as usize, true);
        Self {
            len: self.len,
            bits: self
                .bits
                .iter()
                .zip(universe.bits.iter())
                .map(|(a, b)| !a & b)
                .collect::<Vec<_>>()
                .into(),
        }
    }
}
