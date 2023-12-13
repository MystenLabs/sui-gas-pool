// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasPoolStorageConfig;
use crate::storage::{connect_storage, Storage};
use crate::sui_client::SuiClient;
use crate::types::GasCoin;
use parking_lot::Mutex;
use shared_crypto::intent::Intent;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
use sui_types::base_types::{ObjectID, SuiAddress};
use sui_types::coin::{PAY_MODULE_NAME, PAY_SPLIT_N_FUNC_NAME};
use sui_types::crypto::SuiKeyPair;
use sui_types::gas_coin::{GAS, MIST_PER_SUI};
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::transaction::{Argument, Command, Transaction, TransactionData};
use sui_types::SUI_FRAMEWORK_PACKAGE_ID;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, info};

/// We do not split a coin if it's below this balance.
/// This is important because otherwise we may not even afford the gas cost of splitting.
const MIN_SPLIT_COIN_BALANCE: u64 = MIST_PER_SUI;

pub struct GasPoolInitializer {}

#[derive(Clone)]
struct CoinSplitEnv {
    target_init_coin_balance: u64,
    sponsor_address: SuiAddress,
    keypair: Arc<SuiKeyPair>,
    sui_client: SuiClient,
    task_queue: Arc<Mutex<VecDeque<JoinHandle<Vec<GasCoin>>>>>,
    total_coin_count: Arc<AtomicUsize>,
}

impl CoinSplitEnv {
    fn enqueue_task(&self, coin: GasCoin) -> Option<GasCoin> {
        if coin.balance <= MIN_SPLIT_COIN_BALANCE + self.target_init_coin_balance {
            debug!(
                "Skip splitting coin {:?} because it has small balance",
                coin
            );
            return Some(coin);
        }
        let env = self.clone();
        let task = tokio::task::spawn(async move { env.split_one_gas_coin(coin).await });
        self.task_queue.lock().push_back(task);
        None
    }

    fn increment_total_coin_count_by(&self, delta: usize) {
        info!(
            "Number of coins got so far: {}",
            self.total_coin_count
                .fetch_add(delta, std::sync::atomic::Ordering::Relaxed)
                + delta
        );
    }

    async fn split_one_gas_coin(self, coin: GasCoin) -> Vec<GasCoin> {
        let rgp = self.sui_client.get_reference_gas_price().await;
        let mut pt_builder = ProgrammableTransactionBuilder::new();
        let split_off_count =
            (coin.balance - MIN_SPLIT_COIN_BALANCE) / self.target_init_coin_balance;
        if split_off_count > 500 {
            debug!("Evenly splitting coin {:?} into 100 coins", coin);
            let pure_arg = pt_builder.pure(100u64).unwrap();
            pt_builder.programmable_move_call(
                SUI_FRAMEWORK_PACKAGE_ID,
                PAY_MODULE_NAME.into(),
                PAY_SPLIT_N_FUNC_NAME.into(),
                vec![GAS::type_tag()],
                vec![Argument::GasCoin, pure_arg],
            );
        } else {
            debug!(
                "Splitting coin {:?} into {:?} coins, with target balance of {}",
                coin,
                split_off_count + 1,
                self.target_init_coin_balance,
            );
            let amount_argument = pt_builder.pure(self.target_init_coin_balance).unwrap();
            let amounts = (0..split_off_count)
                .map(|_| amount_argument)
                .collect::<Vec<_>>();
            let split_coins = pt_builder.command(Command::SplitCoins(Argument::GasCoin, amounts));
            let Argument::Result(split_coin_result_index) = split_coins else {
                panic!("Unexpected split result");
            };
            let self_addr = pt_builder.pure(self.sponsor_address).unwrap();
            let transfer_arguments = (0..split_off_count)
                .map(|i| Argument::NestedResult(split_coin_result_index, i as u16))
                .collect::<Vec<_>>();
            pt_builder.command(Command::TransferObjects(transfer_arguments, self_addr));
        }
        let pt = pt_builder.finish();
        let tx = TransactionData::new_programmable(
            self.sponsor_address,
            vec![coin.object_ref],
            pt,
            MIST_PER_SUI, // 1 SUI
            rgp,
        );
        let tx = Transaction::from_data_and_signer(
            tx,
            Intent::sui_transaction(),
            vec![self.keypair.as_ref()],
        );
        debug!(
            "Sending transaction for execution. Tx digest: {:?}",
            tx.digest()
        );
        let effects = self
            .sui_client
            .execute_transaction(tx.clone(), Duration::from_secs(10))
            .await
            .expect("Failed to execute transaction after retries, give up");
        assert!(
            effects.status().is_ok(),
            "Transaction failed. This should never happen. Tx: {:?}, effects: {:?}",
            tx,
            effects
        );
        let all_changed: Vec<ObjectID> = effects
            .all_changed_objects()
            .into_iter()
            .map(|(oref, _)| oref.reference.object_id)
            .collect();
        self.increment_total_coin_count_by(all_changed.len() - 1);
        let updated = self.sui_client.get_latest_gas_objects(&all_changed).await;
        assert!(updated.deleted_gas_coins.is_empty());
        let mut result = vec![];
        for coin in updated.live_gas_coins {
            result.extend(self.enqueue_task(coin));
        }
        result
    }
}

impl GasPoolInitializer {
    async fn split_gas_coins(coins: Vec<GasCoin>, env: CoinSplitEnv) -> Vec<GasCoin> {
        let total_balance: u64 = coins.iter().map(|c| c.balance).sum();
        info!(
            "Splitting {} coins with total balance of {} into smaller coins with target balance of {}. This will result in close to {} coins",
            coins.len(),
            total_balance,
            env.target_init_coin_balance,
            total_balance / env.target_init_coin_balance,
        );
        let mut result = vec![];
        for coin in coins {
            result.extend(env.enqueue_task(coin));
        }
        loop {
            let Some(task) = env.task_queue.lock().pop_front() else {
                break;
            };
            result.extend(task.await.unwrap());
        }
        info!("Splitting finished. Got {} coins", result.len());
        result
    }

    pub async fn run(
        fullnode_url: &str,
        gas_pool_config: &GasPoolStorageConfig,
        target_init_coin_balance: u64,
        keypair: Arc<SuiKeyPair>,
    ) -> Arc<dyn Storage> {
        let sui_client = SuiClient::new(fullnode_url).await;
        let storage = connect_storage(gas_pool_config).await;
        let sponsor_address = (&keypair.public()).into();
        let coins = sui_client.get_all_owned_sui_coins(sponsor_address).await;
        let total_coin_count = Arc::new(AtomicUsize::new(coins.len()));
        let start = Instant::now();
        let result = Self::split_gas_coins(
            coins,
            CoinSplitEnv {
                target_init_coin_balance,
                sponsor_address,
                keypair,
                sui_client,
                task_queue: Default::default(),
                total_coin_count,
            },
        )
        .await;
        for chunk in result.chunks(5000) {
            storage
                .update_gas_coins(sponsor_address, chunk.to_vec(), vec![])
                .await
                .unwrap();
        }
        info!("Pool initialization took {:?}s", start.elapsed().as_secs());
        storage
    }
}

#[cfg(test)]
mod tests {
    use crate::config::GasStationConfig;
    use crate::gas_pool_initializer::GasPoolInitializer;
    use crate::test_env::start_sui_cluster;
    use std::sync::Arc;
    use sui_types::gas_coin::MIST_PER_SUI;

    // TODO: Add more accurate tests.

    #[tokio::test]
    async fn test_basic_init_flow() {
        telemetry_subscribers::init_for_testing();
        let (_cluster, config) = start_sui_cluster(vec![1000 * MIST_PER_SUI]).await;
        let GasStationConfig {
            keypair,
            gas_pool_config,
            fullnode_url,
            ..
        } = config;
        let sponsor = (&keypair.public()).into();
        let keypair = Arc::new(keypair);
        let storage = GasPoolInitializer::run(
            fullnode_url.as_str(),
            &gas_pool_config,
            MIST_PER_SUI,
            keypair,
        )
        .await;
        assert!(storage.get_available_coin_count(sponsor).await > 900);
    }

    #[tokio::test]
    async fn test_init_non_even_split() {
        telemetry_subscribers::init_for_testing();
        let (_cluster, config) = start_sui_cluster(vec![10000000 * MIST_PER_SUI]).await;
        let GasStationConfig {
            keypair,
            gas_pool_config,
            fullnode_url,
            ..
        } = config;
        let sponsor = (&keypair.public()).into();
        let keypair = Arc::new(keypair);
        let target_init_coin_balance = 12345 * MIST_PER_SUI;
        let storage = GasPoolInitializer::run(
            fullnode_url.as_str(),
            &gas_pool_config,
            target_init_coin_balance,
            keypair,
        )
        .await;
        assert!(storage.get_available_coin_count(sponsor).await > 800);
    }
}
