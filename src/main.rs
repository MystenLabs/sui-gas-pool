// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use sui_gas_station::command::Command;

#[tokio::main]
async fn main() {
    let command = Command::parse();
    command.execute().await;
}
