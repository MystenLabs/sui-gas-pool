// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::net::Ipv4Addr;
use sui_config::Config;
use sui_types::crypto::{get_account_key_pair, SuiKeyPair};

pub const DEFAULT_RPC_PORT: u16 = 9527;
pub const DEFAULT_METRICS_PORT: u16 = 9528;
pub const LOCALHOST: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct GasStationConfig {
    // TODO: Make this a vector.
    pub keypair: SuiKeyPair,
    pub rpc_host_ip: Ipv4Addr,
    pub rpc_port: u16,
    pub gas_pool_config: GasPoolStorageConfig,
    pub fullnode_url: String,
    pub run_coin_expiring_task: bool,
}

impl Default for GasStationConfig {
    fn default() -> Self {
        let (_, keypair) = get_account_key_pair();
        GasStationConfig {
            keypair: keypair.into(),
            rpc_host_ip: LOCALHOST,
            rpc_port: DEFAULT_RPC_PORT,
            gas_pool_config: GasPoolStorageConfig::default(),
            fullnode_url: "http://localhost:9000".to_string(),
            run_coin_expiring_task: true,
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
            redis_url: "redis://127.0.0.1/".to_string(),
        }
    }
}

impl Config for GasStationConfig {}
