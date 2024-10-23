// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::tx_signer::{SidecarTxSigner, TestTxSigner, TxSigner};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::net::Ipv4Addr;
use std::sync::Arc;
use sui_config::Config;
use sui_types::crypto::{get_account_key_pair, SuiKeyPair};
use sui_types::gas_coin::MIST_PER_SUI;

pub const DEFAULT_RPC_PORT: u16 = 9527;
pub const DEFAULT_METRICS_PORT: u16 = 9184;
// 0.1 SUI.
pub const DEFAULT_INIT_COIN_BALANCE: u64 = MIST_PER_SUI / 10;
// 24 hours.
const DEFAULT_COIN_POOL_REFRESH_INTERVAL_SEC: u64 = 60 * 60 * 24;
pub const DEFAULT_DAILY_GAS_USAGE_CAP: u64 = 1500 * MIST_PER_SUI;

// Use 127.0.0.1 for tests to avoid OS complaining about permissions.
#[cfg(test)]
pub const LOCALHOST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
#[cfg(not(test))]
pub const LOCALHOST: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct GasStationConfig {
    pub signer_config: TxSignerConfig,
    pub rpc_host_ip: Ipv4Addr,
    pub rpc_port: u16,
    pub metrics_port: u16,
    pub gas_pool_config: GasPoolStorageConfig,
    pub fullnode_url: String,
    /// An optional basic auth when connecting to the fullnode. If specified, the format is
    /// (username, password).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullnode_basic_auth: Option<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coin_init_config: Option<CoinInitConfig>,
    pub daily_gas_usage_cap: u64,
}

impl Config for GasStationConfig {}

impl Default for GasStationConfig {
    fn default() -> Self {
        GasStationConfig {
            signer_config: TxSignerConfig::default(),
            rpc_host_ip: LOCALHOST,
            rpc_port: DEFAULT_RPC_PORT,
            metrics_port: DEFAULT_METRICS_PORT,
            gas_pool_config: GasPoolStorageConfig::default(),
            fullnode_url: "http://localhost:9000".to_string(),
            fullnode_basic_auth: None,
            coin_init_config: Some(CoinInitConfig::default()),
            daily_gas_usage_cap: DEFAULT_DAILY_GAS_USAGE_CAP,
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

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TxSignerConfig {
    Local { keypair: SuiKeyPair },
    Sidecar { sidecar_url: String },
    MultiSidecar { sidecar_urls: Vec<String> },
}

impl Default for TxSignerConfig {
    fn default() -> Self {
        let (_, keypair) = get_account_key_pair();
        Self::Local {
            keypair: keypair.into(),
        }
    }
}

impl TxSignerConfig {
    pub async fn new_signer(self) -> Arc<dyn TxSigner> {
        match self {
            TxSignerConfig::Local { keypair } => TestTxSigner::new(keypair),
            TxSignerConfig::Sidecar { sidecar_url } => {
                SidecarTxSigner::new(vec![sidecar_url]).await
            }
            TxSignerConfig::MultiSidecar { sidecar_urls } => {
                SidecarTxSigner::new(sidecar_urls).await
            }
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CoinInitConfig {
    /// When we split a new gas coin, what is the target balance for the new coins, in MIST.
    pub target_init_balance: u64,
    /// How often do we look at whether there are new coins added to the sponsor account that
    /// requires initialization, i.e. splitting into smaller coins and add them to the gas pool.
    /// This is in seconds.
    pub refresh_interval_sec: u64,
}

impl Default for CoinInitConfig {
    fn default() -> Self {
        CoinInitConfig {
            target_init_balance: DEFAULT_INIT_COIN_BALANCE,
            refresh_interval_sec: DEFAULT_COIN_POOL_REFRESH_INTERVAL_SEC,
        }
    }
}
