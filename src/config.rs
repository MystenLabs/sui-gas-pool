// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::path::PathBuf;
use sui_config::Config;
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{get_account_key_pair, AccountKeyPair};

pub const DEFAULT_RPC_PORT: u16 = 9527;

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct GasStationConfig {
    pub sponsor_address: SuiAddress,
    pub keypair: AccountKeyPair,
    pub local_db_path: PathBuf,
    pub rpc_port: u16,
    pub gas_pool_config: GasPoolStorageConfig,
    pub fullnode_url: String,
}

impl Default for GasStationConfig {
    fn default() -> Self {
        let (sponsor_address, keypair) = get_account_key_pair();
        GasStationConfig {
            sponsor_address,
            keypair,
            local_db_path: tempfile::tempdir().unwrap().into_path(),
            rpc_port: DEFAULT_RPC_PORT,
            gas_pool_config: GasPoolStorageConfig::default(),
            fullnode_url: "http://localhost:9000".to_string(),
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GasPoolStorageConfig {
    RocksDb { db_path: PathBuf },
}

impl Default for GasPoolStorageConfig {
    fn default() -> Self {
        GasPoolStorageConfig::RocksDb {
            db_path: tempfile::tempdir().unwrap().into_path(),
        }
    }
}

impl Config for GasStationConfig {}
