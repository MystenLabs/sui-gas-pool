// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::*;
use std::path::PathBuf;
use sui_config::Config;
use sui_gas_station::benchmarks::run_benchmark;
use sui_gas_station::config::{GasPoolStorageConfig, GasStationConfig};
use sui_gas_station::rpc::client::GasPoolRpcClient;

#[derive(Parser)]
#[command(
    name = "sui-gas-pool-tool",
    about = "Sui Gas Pool Command Line Tools",
    rename_all = "kebab-case"
)]
pub enum ToolCommand {
    /// Running benchmark. This will continue reserving gas coins on the gas station for some
    /// seconds, which would automatically expire latter.
    #[clap(name = "benchmark")]
    Benchmark {
        #[arg(long, help = "Full URL to the gas station RPC server")]
        gas_station_url: String,
        #[arg(
            long,
            help = "Average duration for each reservation, in number of seconds.",
            default_value_t = 1
        )]
        reserve_duration_sec: u64,
        #[arg(
            long,
            help = "Number of clients to spawn to send requests to servers.",
            default_value_t = 100
        )]
        num_clients: u64,
    },
    /// Generate a sample config file and put it in the specified path.
    #[clap(name = "generate-sample-config")]
    GenerateSampleConfig {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
    },
    #[clap(name = "cli")]
    CLI {
        #[clap(subcommand)]
        cli_command: CliCommand,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum BenchmarkMode {
    ReserveOnly,
}

#[derive(Subcommand)]
pub enum CliCommand {
    CheckStationHealth {
        #[clap(long, help = "Full URL of the station RPC server")]
        station_rpc_url: String,
    },
}

impl ToolCommand {
    pub async fn execute(self) {
        match self {
            ToolCommand::Benchmark {
                gas_station_url,
                reserve_duration_sec,
                num_clients,
            } => {
                assert!(
                    cfg!(not(debug_assertions)),
                    "Benchmark should only run in release build"
                );
                run_benchmark(gas_station_url, reserve_duration_sec, num_clients).await
            }
            ToolCommand::GenerateSampleConfig { config_path } => {
                let config = GasStationConfig {
                    gas_pool_config: GasPoolStorageConfig::Redis {
                        redis_url: "redis:://127.0.0.1".to_string(),
                    },
                    ..Default::default()
                };
                config.save(config_path).unwrap();
            }
            ToolCommand::CLI { cli_command } => match cli_command {
                CliCommand::CheckStationHealth { station_rpc_url } => {
                    let station_client = GasPoolRpcClient::new(station_rpc_url);
                    station_client.check_health().await.unwrap();
                    println!("Station server is healthy");
                }
            },
        }
    }
}

#[tokio::main]
async fn main() {
    let command = ToolCommand::parse();
    command.execute().await;
}
