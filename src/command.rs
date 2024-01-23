// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::benchmarks::run_benchmark;
use crate::config::{GasPoolStorageConfig, GasStationConfig};
use crate::gas_pool::gas_pool_core::GasPoolContainer;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::metrics::{GasPoolCoreMetrics, GasPoolRpcMetrics, StorageMetrics};
use crate::rpc::client::GasPoolRpcClient;
use crate::rpc::GasPoolServer;
use crate::storage::connect_storage;
use clap::*;
use prometheus::Registry;
use std::path::PathBuf;
use std::sync::Arc;
use sui_config::Config;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "sui-gas-station",
    about = "Sui Gas Station",
    rename_all = "kebab-case"
)]
pub enum Command {
    /// Start a local gas station instance listening on RPC.
    #[clap(name = "start-station-server")]
    StartStation {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
        #[arg(
            long,
            help = "RPC port to listen on for prometheus metrics",
            default_value_t = 9184
        )]
        metrics_port: u16,
        #[arg(
            long,
            help = "If specified, run the gas pool initialization process. This should only run once globally for each \
            address, and it will take some time to finish. It looks at all the gas coins currently owned by the provided sponsor address, split them into
            smaller gas coins with specified target balance, and initialize the gas pool with these coins."
        )]
        force_init_gas_pool_target_balance: Option<u64>,
    },
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

impl Command {
    pub fn get_metrics_port(&self) -> Option<u16> {
        match self {
            Command::StartStation { metrics_port, .. } => Some(*metrics_port),
            _ => None,
        }
    }
    pub async fn execute(self, prometheus_registry: Option<Registry>) {
        match self {
            Command::StartStation {
                config_path,
                metrics_port: _,
                force_init_gas_pool_target_balance,
            } => {
                let config: GasStationConfig = GasStationConfig::load(config_path).unwrap();
                info!("Config: {:?}", config);
                let GasStationConfig {
                    gas_pool_config,
                    fullnode_url,
                    keypair,
                    rpc_host_ip,
                    rpc_port,
                    run_coin_expiring_task,
                } = config;
                let keypair = Arc::new(keypair);
                let storage_metrics = StorageMetrics::new(prometheus_registry.as_ref().unwrap());
                let storage = connect_storage(&gas_pool_config, storage_metrics).await;
                if let Some(target_init_coin_balance) = force_init_gas_pool_target_balance {
                    GasPoolInitializer::run(
                        fullnode_url.as_str(),
                        &storage,
                        target_init_coin_balance,
                        keypair.clone(),
                    )
                    .await;
                }

                let core_metrics = GasPoolCoreMetrics::new(prometheus_registry.as_ref().unwrap());
                let container = GasPoolContainer::new(
                    keypair,
                    storage,
                    &fullnode_url,
                    run_coin_expiring_task,
                    core_metrics,
                )
                .await;

                let rpc_metrics = GasPoolRpcMetrics::new(prometheus_registry.as_ref().unwrap());
                let server = GasPoolServer::new(
                    container.get_gas_pool_arc(),
                    rpc_host_ip,
                    rpc_port,
                    rpc_metrics,
                )
                .await;
                server.handle.await.unwrap();
            }
            Command::Benchmark {
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
            Command::GenerateSampleConfig { config_path } => {
                let config = GasStationConfig {
                    gas_pool_config: GasPoolStorageConfig::Redis {
                        redis_url: "redis:://127.0.0.1".to_string(),
                    },
                    ..Default::default()
                };
                config.save(config_path).unwrap();
            }
            Command::CLI { cli_command } => match cli_command {
                CliCommand::CheckStationHealth { station_rpc_url } => {
                    let station_client = GasPoolRpcClient::new(station_rpc_url);
                    station_client.check_health().await.unwrap();
                    println!("Station server is healthy");
                }
            },
        }
    }
}
