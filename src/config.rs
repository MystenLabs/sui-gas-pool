// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::net::Ipv4Addr;
use sui_config::Config;
use sui_types::crypto::{get_account_key_pair, SuiKeyPair};
use sui_types::gas_coin::MIST_PER_SUI;

pub const DEFAULT_RPC_PORT: u16 = 9527;
pub const DEFAULT_METRICS_PORT: u16 = 9184;
// 0.1 SUI.
pub const DEFAULT_INIT_COIN_BALANCE: u64 = MIST_PER_SUI / 10;

// Use 127.0.0.1 for tests to avoid OS complaining about permissions.
#[cfg(test)]
pub const LOCALHOST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
#[cfg(not(test))]
pub const LOCALHOST: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct GasStationConfig {
    // TODO: Make this a vector.
    pub keypair: SuiKeyPair,
    pub rpc_host_ip: Ipv4Addr,
    pub rpc_port: u16,
    pub metrics_port: u16,
    pub gas_pool_config: GasPoolStorageConfig,
    pub fullnode_url: String,
    /// Whether to run the demon task that checks for expired reservations.
    /// This should always be enabled in production.
    /// It can be useful to disable this in tests or local testing.
    pub run_coin_expiring_task: bool,
    pub target_init_coin_balance: u64,
}

impl Default for GasStationConfig {
    fn default() -> Self {
        let (_, keypair) = get_account_key_pair();
        GasStationConfig {
            keypair: keypair.into(),
            rpc_host_ip: LOCALHOST,
            rpc_port: DEFAULT_RPC_PORT,
            metrics_port: DEFAULT_METRICS_PORT,
            gas_pool_config: GasPoolStorageConfig::default(),
            fullnode_url: "http://localhost:9000".to_string(),
            run_coin_expiring_task: true,
            target_init_coin_balance: DEFAULT_INIT_COIN_BALANCE,
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GasPoolStorageConfig {
    Redis { redis_url: String },
}

impl Default for GasPoolStorageConfig {
    fn default() -> Self {
        Self::Redis {
            redis_url: "redis://127.0.0.1:6379".to_string(),
        }
    }
}

impl Config for GasStationConfig {}
