// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasStationConfig;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::gas_station::simple_gas_station::SimpleGasStation;
use crate::rpc::GasStationServer;
use crate::storage::connect_storage;
use clap::*;
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
    /// Start a local gas station instance.
    #[clap(name = "start")]
    Start {
        #[arg(long, help = "Path to config file")]
        config_path: PathBuf,
    },
}

impl Command {
    pub async fn execute(self) {
        match self {
            Command::Init {
                config_path,
                target_init_coin_balance,
            } => {
                let config = GasStationConfig::load(&config_path).unwrap();
                info!("Config: {:?}", config);
                let gas_pool_initializer = GasPoolInitializer::new(config).await;
                gas_pool_initializer.run(target_init_coin_balance).await;
            }
            Command::Start { config_path } => {
                let config: GasStationConfig = GasStationConfig::load(config_path).unwrap();
                info!("Config: {:?}", config);
                let station = Arc::new(
                    SimpleGasStation::new(
                        config.sponsor_address,
                        config.keypair,
                        connect_storage(&config.gas_pool_config),
                        &config.fullnode_url,
                    )
                    .await,
                );
                let server = GasStationServer::new(station, config.rpc_port).await;
                server.handle.await.unwrap();
            }
        }
    }
}
