// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasStationConfig;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::gas_station::simple_gas_station::SimpleGasStation;
use std::sync::Arc;
use sui_swarm_config::genesis_config::AccountConfig;
use sui_types::crypto::get_account_key_pair;
use sui_types::gas_coin::MIST_PER_SUI;
use test_cluster::{TestCluster, TestClusterBuilder};

pub async fn start_sui_cluster(init_gas_amounts: Vec<u64>) -> (TestCluster, GasStationConfig) {
    let (sponsor, keypair) = get_account_key_pair();
    let cluster = TestClusterBuilder::new()
        .with_accounts(vec![AccountConfig {
            address: Some(sponsor),
            gas_amounts: init_gas_amounts,
        }])
        .build()
        .await;
    let fullnode_url = cluster.fullnode_handle.rpc_url.clone();
    let config = GasStationConfig {
        sponsor_address: sponsor,
        keypair,
        fullnode_url,
        ..Default::default()
    };
    (cluster, config)
}

pub async fn start_gas_station(init_gas_amounts: Vec<u64>) -> (TestCluster, Arc<SimpleGasStation>) {
    let (test_cluster, config) = start_sui_cluster(init_gas_amounts).await;
    let init = GasPoolInitializer::new(config).await;
    let storage = init.run(MIST_PER_SUI).await;
    let config = init.into_config();
    let station = Arc::new(
        SimpleGasStation::new(
            config.sponsor_address,
            config.keypair,
            storage,
            config.fullnode_url.as_str(),
        )
        .await,
    );
    (test_cluster, station)
}
