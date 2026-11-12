//! BLAS 链下文件构建、查询及独立验证命令；输入输出均为显式 JSON 文件。
use argus_node::{
    model::{metadata::LogMetadata, query::LogQuery},
    query::abi::QueryMatch,
    schemes::blas::{
        file::{FileInput, FileProof, LogFile},
        proof::{QueryProof, TrustedHeader},
    },
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{error::Error, fs, path::Path};

#[derive(Deserialize)]
struct ChainInput {
    headers: Vec<TrustedHeader>,
    query: LogQuery,
    proof: QueryProof,
    matches: Vec<QueryMatch>,
}

/// 从 path 读取 JSON；输入上限限制损坏或意外大文件的内存占用。
fn read<T: DeserializeOwned>(path: &str) -> Result<T, Box<dyn Error>> {
    if fs::metadata(path)?.len() > 32 * 1024 * 1024 {
        return Err("输入超过 32 MiB".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// 将 value 写入指定 path；创建的目录由调用者明确提供。
fn write(path: &str, value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

/// build 构建文件；query 生成 VO；verify 与 verify-chain 不读取服务端完整索引。
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("build") if args.len() == 4 => {
            let file = LogFile::build(read::<FileInput>(&args[2])?)?;
            write(&args[3], &file)?;
            println!("{}", serde_json::to_string(&file.metadata)?);
        }
        Some("query") if args.len() == 5 => {
            let query = read::<LogQuery>(&args[3])?.normalize()?;
            write(&args[4], &read::<LogFile>(&args[2])?.prove(&query)?)?;
        }
        Some("verify") if args.len() == 5 => {
            let query = read::<LogQuery>(&args[3])?.normalize()?;
            let rows = read::<FileProof>(&args[4])?.verify(&read::<LogMetadata>(&args[2])?, &query)?;
            println!("{}", serde_json::to_string(&rows)?);
        }
        Some("verify-chain") if args.len() == 3 => {
            let input: ChainInput = read(&args[2])?;
            let records = input.proof.verify_response(&input.headers, &input.query.normalize()?, &input.matches)?;
            println!("{}", serde_json::to_string(&records)?);
        }
        _ => return Err("用法：blas-file build INPUT OUTPUT | query FILE QUERY VO | verify METADATA QUERY VO | verify-chain INPUT".into()),
    }
    Ok(())
}
