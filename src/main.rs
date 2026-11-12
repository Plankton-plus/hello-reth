//! 唯一节点入口：注册项目自定义的索引节点类型，并在数据库初始化时机将项目自定义的表写入数据库，然后启动节点。固定套路无需动

use argus_node::node::{
    chainspec::IndexedChainSpecParser, config::IndexedNode, storage::IndexTables,
};
use clap::Parser;
use reth_cli_commands::launcher::FnLauncher;
use reth_cli_runner::CliRunner;
use reth_ethereum::{
    cli::interface::{Cli, Commands},
    provider::db::DatabaseEnv,
};

/// 启动实验链；命令行由 Reth CLI 解析。
fn main() -> eyre::Result<()> {
    let mut cli = Cli::<IndexedChainSpecParser>::parse();
    let runner = CliRunner::try_default_runtime()?;
    let _guards = cli.init_tracing(&runner, Default::default())?;
    let Commands::Node(command) = cli.command else {
        eyre::bail!("本实验程序只提供 node 子命令");
    };
    runner.run_command_until_exit(|context| {
        command.execute(
            context,
            FnLauncher::new::<IndexedChainSpecParser, _>(async move |builder, _| {
                let handle = builder
                    .node(IndexedNode)
                    .on_component_initialized(|node| {
                        let mut db: DatabaseEnv = node.provider.as_ref().clone();
                        db.create_and_track_tables_for::<IndexTables>()?;
                        Ok(())
                    })
                    .launch_with_debug_capabilities()
                    .await?;
                handle.wait_for_node_exit().await
            }),
        )
    })
}
