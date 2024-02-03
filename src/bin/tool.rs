// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::*;
use std::path::PathBuf;
use sui_config::Config;
use sui_gas_station::benchmarks::BenchmarkMode;
use sui_gas_station::config::{GasPoolStorageConfig, GasStationConfig, TxSignerConfig};
use sui_gas_station::rpc::client::GasPoolRpcClient;
use sui_types::crypto::get_account_key_pair;

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
            default_value_t = 20
        )]
        reserve_duration_sec: u64,
        #[arg(
            long,
            help = "Number of clients to spawn to send requests to servers.",
            default_value_t = 100
        )]
        num_clients: u64,
        #[arg(long, help = "Benchmark mode.", default_value = "reserve-only")]
        benchmark_mode: BenchmarkMode,
    },
    /// Generate a sample config file and put it in the specified path.
    #[clap(name = "generate-sample-config")]
    GenerateSampleConfig {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
        #[arg(long, help = "Whether to use a sidecar service to sign transactions")]
        with_sidecar_signer: bool,
    },
    #[clap(name = "cli")]
    CLI {
        #[clap(subcommand)]
        cli_command: CliCommand,
    },
}

#[derive(Subcommand)]
pub enum CliCommand {
    CheckStationHealth {
        #[clap(long, help = "Full URL of the station RPC server")]
        station_rpc_url: String,
    },
    GetStationVersion {
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
                benchmark_mode,
            } => {
                assert!(
                    cfg!(not(debug_assertions)),
                    "Benchmark should only run in release build"
                );
                benchmark_mode
                    .run_benchmark(gas_station_url, reserve_duration_sec, num_clients)
                    .await
            }
            ToolCommand::GenerateSampleConfig {
                config_path,
                with_sidecar_signer,
            } => {
                let signer_config = if with_sidecar_signer {
                    TxSignerConfig::Sidecar {
                        sponsor_address: get_account_key_pair().0,
                        sidecar_url: "http://localhost:3000/sign-transaction".to_string(),
                    }
                } else {
                    TxSignerConfig::Local {
                        keypair: get_account_key_pair().1.into(),
                    }
                };
                let config = GasStationConfig {
                    signer_config,
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
                CliCommand::GetStationVersion { station_rpc_url } => {
                    let station_client = GasPoolRpcClient::new(station_rpc_url);
                    let version = station_client.version().await.unwrap();
                    println!("Station server version: {}", version);
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
