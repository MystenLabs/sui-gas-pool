// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasStationConfig;
use crate::gas_pool::gas_pool_core::GasPoolContainer;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::metrics::{GasPoolCoreMetrics, GasPoolRpcMetrics, StorageMetrics};
use crate::rpc::GasPoolServer;
use crate::storage::connect_storage;
use crate::sui_client::SuiClient;
use clap::*;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use sui_config::Config;
use tracing::{error, info};

#[derive(Parser)]
#[command(
    name = "sui-gas-station",
    about = "Sui Gas Station",
    rename_all = "kebab-case"
)]
pub struct Command {
    #[arg(long, help = "Path to config file")]
    config_path: PathBuf,
}

impl Command {
    pub async fn execute(self) {
        let config: GasStationConfig = GasStationConfig::load(self.config_path).unwrap();
        print!("Config: {:?}", config);
        let GasStationConfig {
            signer_config,
            gas_pool_config,
            fullnode_url,
            fullnode_basic_auth,
            rpc_host_ip,
            rpc_port,
            metrics_port,
            coin_init_config,
            daily_gas_usage_cap,
            max_sui_per_request,
            advanced_faucet_mode,
        } = config;

        let metric_address = SocketAddr::new(IpAddr::V4(rpc_host_ip), metrics_port);
        let registry_service = mysten_metrics::start_prometheus_server(metric_address);
        let prometheus_registry = registry_service.default_registry();
        let telemetry_config = telemetry_subscribers::TelemetryConfig::new()
            .with_log_level("off,sui_gas_station=debug")
            .with_env()
            .with_prom_registry(&prometheus_registry);
        let _guard = telemetry_config.init();
        info!("Metrics server started at {:?}", metric_address);

        let signer = signer_config.new_signer().await;
        let storage_metrics = StorageMetrics::new(&prometheus_registry);
        let sponsor_address = signer.get_address();
        info!("Sponsor address: {:?}", sponsor_address);

        let storage =
            match connect_storage(&gas_pool_config, sponsor_address, storage_metrics).await {
                Ok(storage) => storage,
                Err(e) => {
                    error!("Connecting to storage failed with: {e:?}");
                    std::process::exit(1);
                }
            };

        let sui_client = SuiClient::new(&fullnode_url, fullnode_basic_auth).await;
        let target_init_coin_balance = coin_init_config
            .as_ref()
            .map(|config| config.target_init_balance);
        let _coin_init_task = if let Some(coin_init_config) = coin_init_config {
            let task = GasPoolInitializer::start(
                sui_client.clone(),
                storage.clone(),
                coin_init_config,
                signer.clone(),
            )
            .await;
            Some(task)
        } else {
            None
        };

        let core_metrics = GasPoolCoreMetrics::new(&prometheus_registry);
        let container = GasPoolContainer::new(
            signer,
            storage,
            sui_client,
            daily_gas_usage_cap,
            core_metrics,
            advanced_faucet_mode,
            target_init_coin_balance,
        )
        .await;

        let rpc_metrics = GasPoolRpcMetrics::new(&prometheus_registry);
        let server = GasPoolServer::new(
            container.get_gas_pool_arc(),
            rpc_host_ip,
            rpc_port,
            rpc_metrics,
            max_sui_per_request,
        )
        .await;
        server.handle.await.unwrap();
    }
}
