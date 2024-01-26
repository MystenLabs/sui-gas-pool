// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasStationConfig;
use crate::gas_pool::gas_pool_core::GasPoolContainer;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::metrics::{GasPoolCoreMetrics, GasPoolRpcMetrics, StorageMetrics};
use crate::rpc::GasPoolServer;
use crate::storage::connect_storage;
use clap::*;
use std::net::{IpAddr, SocketAddr};
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
pub struct Command {
    #[arg(long, help = "Path to config file")]
    config_path: PathBuf,
    #[arg(
        long,
        help = "If specified, run the gas pool initialization process. This should only run once globally for each \
            address, and it will take some time to finish. It looks at all the gas coins currently owned by the provided sponsor address, split them into
            smaller gas coins with specified target balance, and initialize the gas pool with these coins."
    )]
    force_init_gas_pool: bool,
}

impl Command {
    pub async fn execute(self) {
        let config: GasStationConfig = GasStationConfig::load(self.config_path).unwrap();
        info!("Config: {:?}", config);
        let GasStationConfig {
            gas_pool_config,
            fullnode_url,
            keypair,
            rpc_host_ip,
            rpc_port,
            metrics_port,
            run_coin_expiring_task,
            target_init_coin_balance,
        } = config;

        let metric_address = SocketAddr::new(IpAddr::V4(rpc_host_ip), metrics_port);
        let registry_service = mysten_metrics::start_prometheus_server(metric_address);
        let prometheus_registry = registry_service.default_registry();
        info!("Metrics server started at {:?}", metric_address);
        let telemetry_config = telemetry_subscribers::TelemetryConfig::new()
            .with_log_level("off,sui_gas_station=debug")
            .with_env()
            .with_prom_registry(&prometheus_registry);
        let _guard = telemetry_config.init();

        let keypair = Arc::new(keypair);
        let storage_metrics = StorageMetrics::new(&prometheus_registry);
        let storage = connect_storage(&gas_pool_config, storage_metrics).await;
        GasPoolInitializer::run(
            fullnode_url.as_str(),
            &storage,
            self.force_init_gas_pool,
            target_init_coin_balance,
            keypair.clone(),
        )
        .await;

        let core_metrics = GasPoolCoreMetrics::new(&prometheus_registry);
        let container = GasPoolContainer::new(
            keypair,
            storage,
            &fullnode_url,
            run_coin_expiring_task,
            core_metrics,
        )
        .await;

        let rpc_metrics = GasPoolRpcMetrics::new(&prometheus_registry);
        let server = GasPoolServer::new(
            container.get_gas_pool_arc(),
            rpc_host_ip,
            rpc_port,
            rpc_metrics,
        )
        .await;
        server.handle.await.unwrap();
    }
}
