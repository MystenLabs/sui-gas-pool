// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::tx_signer::{SidecarTxSigner, TestTxSigner, TxSigner};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::net::Ipv4Addr;
use std::sync::Arc;
use sui_config::Config;
use sui_types::crypto::{SuiKeyPair, get_account_key_pair};
use sui_types::gas_coin::MIST_PER_SUI;

pub const DEFAULT_RPC_PORT: u16 = 9527;
pub const DEFAULT_METRICS_PORT: u16 = 9184;
// 0.1 SUI.
pub const DEFAULT_INIT_COIN_BALANCE: u64 = MIST_PER_SUI / 10;
// 24 hours.
const DEFAULT_COIN_POOL_REFRESH_INTERVAL_SEC: u64 = 60 * 60 * 24;
pub const DEFAULT_DAILY_GAS_USAGE_CAP: u64 = 1500 * MIST_PER_SUI;
// 2 SUI.
pub const DEFAULT_MAX_SUI_PER_REQUEST: u64 = 2 * MIST_PER_SUI;

// Default Redis connection settings
const DEFAULT_REDIS_CONNECTION_TIMEOUT_MS: u64 = 5000;
const DEFAULT_REDIS_RESPONSE_TIMEOUT_MS: u64 = 5000;

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
    /// Maximum SUI allowed per reservation request, in MIST.
    /// Defaults to 2 SUI (2_000_000_000 MIST).
    pub max_sui_per_request: u64,
    /// Enables the gas pool to work as a faucet, where the sender is the same as the sponsor.
    /// Do not set to true unless you have a specific or niche use case and you understand the
    /// risks associated with this mode.
    pub advanced_faucet_mode: bool,
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
            max_sui_per_request: DEFAULT_MAX_SUI_PER_REQUEST,
            advanced_faucet_mode: false,
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GasPoolStorageConfig {
    Redis {
        redis_url: String,
        #[serde(default)]
        connection: RedisConnectionConfig,
    },
}

impl Default for GasPoolStorageConfig {
    fn default() -> Self {
        Self::Redis {
            redis_url: "redis://127.0.0.1:6379".to_string(),
            connection: RedisConnectionConfig::default(),
        }
    }
}

/// Configuration for Redis connection behavior.
/// These settings help prevent "broken pipe" errors by configuring timeouts and keepalive.
#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct RedisConnectionConfig {
    /// Connection timeout in milliseconds.
    /// How long to wait when establishing a new connection to Redis.
    #[serde(default = "default_connection_timeout_ms")]
    pub connection_timeout_ms: u64,

    /// Response timeout in milliseconds.
    /// How long to wait for a response from Redis after sending a command.
    #[serde(default = "default_response_timeout_ms")]
    pub response_timeout_ms: u64,

    /// Number of retries for failed operations.
    #[serde(default = "default_number_of_retries")]
    pub number_of_retries: usize,

    /// Maximum delay between retries in milliseconds.
    #[serde(default = "default_max_retry_delay_ms")]
    pub max_retry_delay_ms: u64,

    /// Exponential backoff factor for retries.
    #[serde(default = "default_retry_factor")]
    pub retry_factor: u64,

    /// TCP keepalive interval in seconds.
    /// Sends periodic TCP keepalive probes to prevent idle connections from being
    /// terminated by firewalls, NAT gateways, or load balancers.
    /// Set to 0 to disable. Recommended: 60 seconds for most cloud environments.
    #[serde(default = "default_tcp_keepalive_secs")]
    pub tcp_keepalive_secs: u64,
}

fn default_connection_timeout_ms() -> u64 {
    DEFAULT_REDIS_CONNECTION_TIMEOUT_MS
}

fn default_response_timeout_ms() -> u64 {
    DEFAULT_REDIS_RESPONSE_TIMEOUT_MS
}

fn default_number_of_retries() -> usize {
    3
}

fn default_max_retry_delay_ms() -> u64 {
    5000
}

fn default_retry_factor() -> u64 {
    2
}

fn default_tcp_keepalive_secs() -> u64 {
    60
}

fn default_max_sui_per_request() -> u64 {
    DEFAULT_MAX_SUI_PER_REQUEST
}

impl Default for RedisConnectionConfig {
    fn default() -> Self {
        Self {
            connection_timeout_ms: DEFAULT_REDIS_CONNECTION_TIMEOUT_MS,
            response_timeout_ms: DEFAULT_REDIS_RESPONSE_TIMEOUT_MS,
            number_of_retries: 3,
            max_retry_delay_ms: 5000,
            retry_factor: 2,
            tcp_keepalive_secs: 60,
        }
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TxSignerConfig {
    Local { keypair: SuiKeyPair },
    Sidecar { sidecar_url: String },
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
            TxSignerConfig::Sidecar { sidecar_url } => SidecarTxSigner::new(sidecar_url).await,
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
