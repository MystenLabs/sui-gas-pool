// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::{CoinInitConfig, DEFAULT_DAILY_GAS_USAGE_CAP};
use crate::gas_pool::gas_pool_core::GasPoolContainer;
use crate::gas_pool_initializer::GasPoolInitializer;
use crate::metrics::{GasPoolCoreMetrics, GasPoolRpcMetrics};
use crate::rpc::GasPoolServer;
use crate::storage::connect_storage_for_testing;
use crate::sui_client::SuiClient;
use crate::tx_signer::{TestTxSigner, TxSigner};
use crate::AUTH_ENV_NAME;
use std::sync::Arc;
use sui_config::local_ip_utils::{get_available_port, localhost_for_testing};
use sui_swarm_config::genesis_config::AccountConfig;
use sui_types::base_types::{ObjectRef, SuiAddress};
use sui_types::crypto::{get_account_key_pair, KeypairTraits, SuiKeyPair};
use sui_types::gas_coin::MIST_PER_SUI;
use sui_types::signature::GenericSignature;
use sui_types::transaction::{TransactionData, TransactionDataAPI};
use test_cluster::{TestCluster, TestClusterBuilder};
use tracing::debug;

pub async fn start_sui_cluster(
    init_gas_amounts: Vec<u64>,
) -> (TestCluster, Arc<dyn TxSigner>, SuiKeyPair) {
    let (sponsor, keypair) = get_account_key_pair();
    let pk = SuiKeyPair::Ed25519(keypair.copy());
    let cluster = TestClusterBuilder::new()
        .with_accounts(vec![
            AccountConfig {
                address: Some(sponsor),
                gas_amounts: init_gas_amounts,
            },
            // Besides sponsor, also initialize another account with 1000 SUI.
            AccountConfig {
                address: None,
                gas_amounts: vec![1000 * MIST_PER_SUI],
            },
        ])
        .build()
        .await;
    (cluster, TestTxSigner::new(keypair.into()), pk)
}

pub async fn start_gas_station(
    init_gas_amounts: Vec<u64>,
    target_init_coin_balance: u64,
    advanced_faucet_mode: bool,
) -> (TestCluster, GasPoolContainer) {
    debug!("Starting Sui cluster..");
    let (test_cluster, signer, _) = start_sui_cluster(init_gas_amounts).await;
    let fullnode_url = test_cluster.fullnode_handle.rpc_url.clone();
    let sponsor_address = signer.get_address();
    debug!("Starting storage. Sponsor address: {:?}", sponsor_address);
    let storage = connect_storage_for_testing(sponsor_address).await;
    let sui_client = SuiClient::new(&fullnode_url, None).await;
    GasPoolInitializer::start(
        sui_client.clone(),
        storage.clone(),
        CoinInitConfig {
            target_init_balance: target_init_coin_balance,
            ..Default::default()
        },
        signer.clone(),
    )
    .await;
    let station = GasPoolContainer::new(
        signer,
        storage,
        sui_client,
        DEFAULT_DAILY_GAS_USAGE_CAP,
        GasPoolCoreMetrics::new_for_testing(),
        advanced_faucet_mode,
    )
    .await;
    (test_cluster, station)
}

pub async fn start_rpc_server_for_testing(
    init_gas_amounts: Vec<u64>,
    target_init_balance: u64,
    advanced_faucet_mode: bool,
) -> (TestCluster, GasPoolContainer, GasPoolServer) {
    let (test_cluster, container) =
        start_gas_station(init_gas_amounts, target_init_balance, advanced_faucet_mode).await;
    let localhost = localhost_for_testing();
    std::env::set_var(AUTH_ENV_NAME, "some secret");
    let server = GasPoolServer::new(
        container.get_gas_pool_arc(),
        localhost.parse().unwrap(),
        get_available_port(&localhost),
        GasPoolRpcMetrics::new_for_testing(),
    )
    .await;
    (test_cluster, container, server)
}

pub async fn create_test_transaction(
    test_cluster: &TestCluster,
    sponsor: SuiAddress,
    gas_coins: Vec<ObjectRef>,
) -> (TransactionData, GenericSignature) {
    let user = test_cluster
        .get_addresses()
        .into_iter()
        .find(|a| *a != sponsor)
        .unwrap();
    let object = test_cluster
        .wallet
        .get_one_gas_object_owned_by_address(user)
        .await
        .unwrap()
        .unwrap();
    let mut tx_data = test_cluster
        .test_transaction_builder_with_gas_object(user, gas_coins[0])
        .await
        .transfer(object, user)
        .build();
    // TODO: Add proper sponsored transaction support to test tx builder.
    tx_data.gas_data_mut().payment = gas_coins;
    tx_data.gas_data_mut().owner = sponsor;
    let user_sig = test_cluster
        .sign_transaction(&tx_data)
        .into_data()
        .tx_signatures_mut_for_testing()
        .pop()
        .unwrap();
    (tx_data, user_sig)
}

pub async fn start_gas_station_with_cluster(
    test_cluster: &mut TestCluster,
    signer: Arc<dyn TxSigner>,
    target_init_coin_balance: u64,
    advanced_faucet_mode: bool,
) -> (&mut TestCluster, GasPoolContainer) {
    debug!("Starting Sui cluster..");
    let fullnode_url = test_cluster.fullnode_handle.rpc_url.clone();
    let sponsor_address = signer.get_address();
    debug!("Starting storage. Sponsor address: {:?}", sponsor_address);
    let storage = connect_storage_for_testing(sponsor_address).await;
    let sui_client = SuiClient::new(&fullnode_url, None).await;
    GasPoolInitializer::start(
        sui_client.clone(),
        storage.clone(),
        CoinInitConfig {
            target_init_balance: target_init_coin_balance,
            ..Default::default()
        },
        signer.clone(),
    )
    .await;
    let station = GasPoolContainer::new(
        signer,
        storage,
        sui_client,
        DEFAULT_DAILY_GAS_USAGE_CAP,
        GasPoolCoreMetrics::new_for_testing(),
        advanced_faucet_mode,
    )
    .await;
    (test_cluster, station)
}

pub async fn create_test_transaction_with_same_sender_as_sponsor(
    test_cluster: &mut TestCluster,
    sponsor: SuiAddress,
    keypair: SuiKeyPair,
    gas_coins: Vec<ObjectRef>,
) -> (TransactionData, GenericSignature) {
    let user = sponsor.clone();
    test_cluster
        .wallet_mut()
        .add_account(Some("sponsor".to_string()), keypair);
    let object = test_cluster
        .wallet
        .get_one_gas_object_owned_by_address(user)
        .await
        .unwrap()
        .unwrap();
    let mut tx_data = test_cluster
        .test_transaction_builder_with_gas_object(user, gas_coins[0])
        .await
        .transfer(object, user)
        .build();
    // TODO: Add proper sponsored transaction support to test tx builder.
    tx_data.gas_data_mut().payment = gas_coins;
    tx_data.gas_data_mut().owner = sponsor;
    let user_sig = test_cluster
        .sign_transaction(&tx_data)
        .into_data()
        .tx_signatures_mut_for_testing()
        .pop()
        .unwrap();
    (tx_data, user_sig)
}

pub async fn create_pay_sui_transaction_same_sender_as_sponsor(
    test_cluster: &mut TestCluster,
    sponsor: SuiAddress,
    keypair: SuiKeyPair,
    gas_coins: Vec<ObjectRef>,
) -> (TransactionData, GenericSignature) {
    test_cluster.wallet_mut().add_account(None, keypair);

    let recipient = get_account_key_pair();
    let tx_data = TransactionData::new_pay_sui(
        sponsor,
        gas_coins[1..].to_vec(),
        vec![recipient.0],
        vec![1_000_000_000],
        gas_coins[0],
        10000000,
        test_cluster.get_reference_gas_price().await,
    )
    .unwrap();
    let user_sig = test_cluster
        .sign_transaction(&tx_data)
        .into_data()
        .tx_signatures_mut_for_testing()
        .pop()
        .unwrap();
    (tx_data, user_sig)
}
