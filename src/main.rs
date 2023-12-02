// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use sui_gas_station::command::Command;

#[tokio::main]
async fn main() {
    let _guard = telemetry_subscribers::TelemetryConfig::new()
        .with_log_level("off,sui_gas_station=info")
        .with_env()
        .init();

    let command = Command::parse();
    command.execute().await;
}
