// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasStationConfig;
use crate::storage::{connect_storage, Storage};
use crate::types::GasCoin;
use shared_crypto::intent::Intent;
use std::cmp::min;
use std::sync::Arc;
use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
use sui_sdk::rpc_types::SuiTransactionBlockResponseOptions;
use sui_sdk::{SuiClient, SuiClientBuilder};
use sui_types::gas_coin::MIST_PER_SUI;
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::transaction::{Argument, Command, Transaction, TransactionData};
use tracing::info;

/// We do not split a coin if it's below this balance.
/// This is important because otherwise we may not even afford the gas cost of splitting.
const MIN_SPLIT_COIN_BALANCE: u64 = MIST_PER_SUI;

pub struct GasPoolInitializer {
    config: GasStationConfig,
}

impl GasPoolInitializer {
    pub async fn new(config: GasStationConfig) -> Self {
        Self { config }
    }

    pub fn into_config(self) -> GasStationConfig {
        self.config
    }

    async fn query_all_owned_sui_coins(&self, sui_client: &SuiClient) -> Vec<GasCoin> {
        info!(
            "Querying all gas coins owned by sponsor address: {}",
            self.config.sponsor_address
        );
        let mut cursor = None;
        let mut coins = Vec::new();
        loop {
            let page = sui_client
                .coin_read_api()
                .get_coins(self.config.sponsor_address, None, cursor, None)
                .await
                .unwrap();
            for coin in page.data {
                coins.push(GasCoin {
                    object_ref: coin.object_ref(),
                    balance: coin.balance,
                });
            }
            if page.has_next_page {
                cursor = page.next_cursor;
            } else {
                break;
            }
        }
        info!("List of owned coins: {:?}", coins);
        coins
    }

    async fn split_gas_coins(
        &self,
        sui_client: &SuiClient,
        mut coins: Vec<GasCoin>,
        target_init_coin_balance: u64,
    ) -> Vec<GasCoin> {
        let mut result = vec![];
        let rgp = sui_client
            .read_api()
            .get_reference_gas_price()
            .await
            .unwrap();
        info!("Reference gas price: {}", rgp);
        while let Some(coin) = coins.pop() {
            if coin.balance <= MIN_SPLIT_COIN_BALANCE {
                info!(
                    "Skip splitting coin {:?} because it has small balance",
                    coin
                );
                result.push(coin);
                continue;
            }
            // TODO: We can further optimize this to 2000 by using multiple PT commands.
            let split_off_count = min(
                500,
                (coin.balance - MIN_SPLIT_COIN_BALANCE) / target_init_coin_balance,
            );
            if split_off_count == 0 {
                result.push(coin);
                continue;
            }
            info!(
                "Splitting coin {:?} into {:?} coins, with target balance of {}",
                coin,
                split_off_count + 1,
                target_init_coin_balance,
            );
            let mut pt_builder = ProgrammableTransactionBuilder::new();
            let amount_argument = pt_builder.pure(target_init_coin_balance).unwrap();
            let amounts = (0..split_off_count)
                .map(|_| amount_argument)
                .collect::<Vec<_>>();
            let split_coins = pt_builder.command(Command::SplitCoins(Argument::GasCoin, amounts));
            let Argument::Result(split_coin_result_index) = split_coins else {
                panic!("Unexpected split result");
            };
            let self_addr = pt_builder.pure(self.config.sponsor_address).unwrap();
            let transfer_arguments = (0..split_off_count)
                .map(|i| Argument::NestedResult(split_coin_result_index, i as u16))
                .collect::<Vec<_>>();
            pt_builder.command(Command::TransferObjects(transfer_arguments, self_addr));
            let pt = pt_builder.finish();
            let tx = TransactionData::new_programmable(
                self.config.sponsor_address,
                vec![coin.object_ref],
                pt,
                MIST_PER_SUI, // 1 SUI
                rgp,
            );
            let tx = Transaction::from_data_and_signer(
                tx,
                Intent::sui_transaction(),
                vec![&self.config.keypair],
            );
            info!(
                "Sending transaction for execution. Tx digest: {:?}",
                tx.digest()
            );
            let response = sui_client
                .quorum_driver_api()
                .execute_transaction_block(
                    tx,
                    SuiTransactionBlockResponseOptions::full_content(),
                    None,
                )
                .await
                .unwrap();
            let effects = response.effects.unwrap();
            assert!(
                effects.status().is_ok(),
                "Transaction failed: {:?}",
                effects
            );
            info!("Transaction executed successfully");
            for new_coin in effects.created() {
                result.push(GasCoin {
                    object_ref: new_coin.reference.to_object_ref(),
                    balance: target_init_coin_balance,
                });
            }
            let new_balance = coin.balance
                - target_init_coin_balance * split_off_count
                - effects.gas_cost_summary().gas_used();
            coins.push(GasCoin {
                object_ref: effects.gas_object().reference.to_object_ref(),
                balance: new_balance,
            });
        }
        info!("Coins are split into {} coins", result.len());
        result
    }

    pub async fn run(&self, target_init_coin_balance: u64) -> Arc<dyn Storage> {
        assert!(
            target_init_coin_balance >= MIN_SPLIT_COIN_BALANCE,
            "Target init coin balance is too small"
        );
        let storage = connect_storage(&self.config.gas_pool_config);

        let sui_client = SuiClientBuilder::default()
            .build(&self.config.fullnode_url)
            .await
            .unwrap();
        let coins = self.query_all_owned_sui_coins(&sui_client).await;
        let result = self
            .split_gas_coins(&sui_client, coins, target_init_coin_balance)
            .await;
        storage.add_gas_coins(result);
        storage
    }
}

#[cfg(test)]
mod tests {
    use crate::gas_pool_initializer::GasPoolInitializer;
    use crate::test_env::start_sui_cluster;
    use sui_types::gas_coin::MIST_PER_SUI;

    #[tokio::test]
    async fn test_basic_init_flow() {
        telemetry_subscribers::init_for_testing();
        let (_cluster, config) = start_sui_cluster(vec![1000 * MIST_PER_SUI]).await;
        let initializer = GasPoolInitializer::new(config).await;
        let storage = initializer.run(MIST_PER_SUI).await;
        // All coins should have the target balance which is MIST_PER_SUI, except one because
        // of the gas cost.
        let mut seen_non_target_balance_coin = false;
        loop {
            let mut gas_coins = storage.reserve_gas_coins(1);
            if gas_coins.len() == 0 {
                break;
            }
            assert_eq!(gas_coins.len(), 1);
            let gas_coin = gas_coins.pop().unwrap();
            assert!(gas_coin.balance == MIST_PER_SUI || !seen_non_target_balance_coin);
            seen_non_target_balance_coin |= gas_coin.balance != MIST_PER_SUI;
        }
    }
}
