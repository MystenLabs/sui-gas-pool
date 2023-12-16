// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::benchmarks::benchmark_reserve_only;
use crate::config::{GasPoolStorageConfig, GasStationConfig};
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::gas_station::gas_station_core::GasStationContainer;
use crate::metrics::{GasStationMetrics, StoragePoolMetrics};
use crate::rpc::client::GasStationRpcClient;
use crate::rpc::GasStationServer;
use crate::storage::rocksdb::rocksdb_rpc_client::RocksDbRpcClient;
use crate::storage::rocksdb::rocksdb_rpc_server::RocksDbServer;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::{connect_storage, Storage};
use clap::*;
use prometheus::Registry;
use std::net::Ipv4Addr;
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
    /// Initialize the gas pool. This command should be called only and exactly once globally.
    /// It looks at all the gas coins currently owned by the provided sponsor address, split them into
    /// smaller gas coins with target balance, and initialize the gas pool with these coins.
    /// This need to run before we run any gas station.
    #[clap(name = "init")]
    Init {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
        #[arg(
            long,
            help = "The initial per-coin balance we want to split into, in MIST"
        )]
        target_init_coin_balance: u64,
    },
    /// Start a local gas station instance listening on RPC.
    #[clap(name = "start-station-server")]
    StartStation {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
    },
    /// Start a local gas station instance.
    #[clap(name = "start-storage-server")]
    StartStorage {
        #[arg(long, help = "Path to the storage")]
        db_path: PathBuf,
        #[arg(long, help = "IP address to listen on for storage requests")]
        ip: Ipv4Addr,
        #[arg(long, help = "RPC port to listen on for storage requests")]
        rpc_port: u16,
    },
    /// Running benchmark locally. This only works in release build.
    #[clap(name = "benchmark")]
    Benchmark { mode: BenchmarkMode },
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
    CheckStorageHealth {
        #[clap(long, help = "Full URL of the storage RPC server")]
        storage_rpc_url: String,
    },
    CheckStationHealth {
        #[clap(long, help = "Full URL of the station RPC server")]
        station_rpc_url: String,
    },
}

impl Command {
    pub async fn execute(self, prometheus_registry: Registry) {
        match self {
            Command::Init {
                config_path,
                target_init_coin_balance,
            } => {
                let config = GasStationConfig::load(&config_path).unwrap();
                info!("Config: {:?}", config);
                let GasStationConfig {
                    keypair,
                    gas_pool_config,
                    fullnode_url,
                    ..
                } = config;
                let keypair = Arc::new(keypair);
                GasPoolInitializer::run(
                    fullnode_url.as_str(),
                    &gas_pool_config,
                    target_init_coin_balance,
                    keypair,
                )
                .await;
            }
            Command::StartStation { config_path } => {
                let station_metrics = GasStationMetrics::new(&prometheus_registry);
                let config: GasStationConfig = GasStationConfig::load(config_path).unwrap();
                info!("Config: {:?}", config);
                let GasStationConfig {
                    gas_pool_config,
                    fullnode_url,
                    keypair,
                    ..
                } = config;
                let container = GasStationContainer::new(
                    Arc::new(keypair),
                    connect_storage(&gas_pool_config).await,
                    &fullnode_url,
                    station_metrics.clone(),
                )
                .await;

                let server = GasStationServer::new(
                    container.get_station(),
                    config.rpc_host_ip,
                    config.rpc_port,
                    station_metrics,
                )
                .await;
                server.handle.await.unwrap();
            }
            Command::StartStorage {
                db_path,
                ip,
                rpc_port,
            } => {
                let metrics = StoragePoolMetrics::new(&prometheus_registry);
                let storage = Arc::new(RocksDBStorage::new(db_path.as_path(), metrics));
                let server = RocksDbServer::new(storage, ip, rpc_port).await;
                server.handle.await.unwrap();
            }
            Command::Benchmark { mode: _ } => {
                assert!(
                    cfg!(not(debug_assertions)),
                    "Benchmark should only run in release build"
                );
                benchmark_reserve_only().await
            }
            Command::GenerateSampleConfig { config_path } => {
                let config = GasStationConfig {
                    gas_pool_config: GasPoolStorageConfig::RemoteRocksDb {
                        db_rpc_url: "http://localhost:9528".to_string(),
                    },
                    local_db_path: PathBuf::from("local_db"),
                    ..Default::default()
                };
                config.save(config_path).unwrap();
            }
            Command::CLI { cli_command } => match cli_command {
                CliCommand::CheckStorageHealth { storage_rpc_url } => {
                    let storage_client = RocksDbRpcClient::new(storage_rpc_url);
                    storage_client.check_health().await.unwrap();
                    println!("Storage server is healthy");
                }
                CliCommand::CheckStationHealth { station_rpc_url } => {
                    let station_client = GasStationRpcClient::new(station_rpc_url);
                    station_client.check_health().await.unwrap();
                    println!("Station server is healthy");
                }
            },
        }
    }
}
