// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod benchmarks;
pub mod command;
pub mod config;
pub mod errors;
pub mod gas_pool;
pub mod gas_pool_initializer;
pub mod metrics;
pub mod rpc;
pub mod storage;
pub mod sui_client;
#[cfg(test)]
pub mod test_env;
pub mod tx_signer;
pub mod types;

pub const AUTH_ENV_NAME: &str = "GAS_STATION_AUTH";

pub fn read_auth_env() -> String {
    std::env::var(AUTH_ENV_NAME)
        .ok()
        .unwrap_or_else(|| panic!("{} environment variable must be specified", AUTH_ENV_NAME))
        .parse::<String>()
        .unwrap()
}
