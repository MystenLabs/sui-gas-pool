// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use sui_gas_station::command::Command;
use tracing::info;

#[tokio::main]
async fn main() {
    // TODO: Make the port configurable.
    let metric_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 9184);
    let registry_service = mysten_metrics::start_prometheus_server(metric_address);
    let prometheus_registry = registry_service.default_registry();

    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_log_level("off,sui_gas_station=info")
        .with_env()
        .with_prom_registry(&prometheus_registry)
        .init();
    info!("Metric server started at {}", metric_address);

    let command = Command::parse();
    command.execute(prometheus_registry).await;
}
