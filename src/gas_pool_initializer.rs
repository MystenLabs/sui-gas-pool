// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::CoinInitConfig;
use crate::retry_forever;
use crate::storage::Storage;
use crate::sui_client::SuiClient;
use crate::tx_signer::TxSigner;
use crate::types::GasCoin;
use parking_lot::Mutex;
use std::cmp::min;
use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
use sui_types::coin::{PAY_MODULE_NAME, PAY_SPLIT_N_FUNC_NAME};
use sui_types::gas_coin::GAS;
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::transaction::{Argument, Transaction, TransactionData};
use sui_types::SUI_FRAMEWORK_PACKAGE_ID;
use tap::TapFallible;
use tokio::task::JoinHandle;
use tokio::time::Instant;
#[cfg(not(test))]
use tokio_retry::strategy::FixedInterval;
#[cfg(not(test))]
use tokio_retry::Retry;
use tracing::{debug, error, info};

/// Any coin owned by the sponsor address with balance above target_init_coin_balance * NEW_COIN_BALANCE_FACTOR_THRESHOLD
/// is considered a new coin, and we will try to split it into smaller coins with balance close to target_init_coin_balance.
const NEW_COIN_BALANCE_FACTOR_THRESHOLD: u64 = 200;

#[derive(Clone)]
struct CoinSplitEnv {
    target_init_coin_balance: u64,
    gas_cost_per_object: u64,
    signer: Arc<dyn TxSigner>,
    sui_client: SuiClient,
    task_queue: Arc<Mutex<VecDeque<JoinHandle<Vec<GasCoin>>>>>,
    total_coin_count: Arc<AtomicUsize>,
    rgp: u64,
}

impl CoinSplitEnv {
    fn enqueue_task(&self, coin: GasCoin) -> Option<GasCoin> {
        if coin.balance <= (self.gas_cost_per_object + self.target_init_coin_balance) * 2 {
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

    async fn split_one_gas_coin(self, mut coin: GasCoin) -> Vec<GasCoin> {
        let rgp = self.rgp;
        let split_count = min(
            // Max number of object mutations per transaction is 2048.
            2000,
            coin.balance / (self.gas_cost_per_object + self.target_init_coin_balance),
        );
        debug!(
            "Evenly splitting coin {:?} into {} coins",
            coin, split_count
        );
        let budget = self.gas_cost_per_object * split_count;
        let effects = loop {
            let mut pt_builder = ProgrammableTransactionBuilder::new();
            let pure_arg = pt_builder.pure(split_count).unwrap();
            pt_builder.programmable_move_call(
                SUI_FRAMEWORK_PACKAGE_ID,
                PAY_MODULE_NAME.into(),
                PAY_SPLIT_N_FUNC_NAME.into(),
                vec![GAS::type_tag()],
                vec![Argument::GasCoin, pure_arg],
            );
            let pt = pt_builder.finish();
            let sponsor_address = self.signer.get_address();
            let tx_data = TransactionData::new_programmable(
                sponsor_address,
                vec![coin.object_ref],
                pt,
                budget,
                rgp,
            );
            let sig = retry_forever!(async {
                self.signer
                    .sign_transaction(&tx_data)
                    .await
                    .tap_err(|err| error!("Failed to sign transaction: {:?}", err))
            })
            .unwrap();
            let tx = Transaction::from_generic_sig_data(tx_data, vec![sig]);
            debug!(
                "Sending transaction for execution. Tx digest: {:?}",
                tx.digest()
            );
            let result = self
                .sui_client
                .execute_transaction(tx.clone(), Duration::from_secs(20))
                .await;
            match result {
                Ok(effects) => {
                    assert!(
                        effects.status().is_ok(),
                        "Transaction failed. This should never happen. Tx: {:?}, effects: {:?}",
                        tx,
                        effects
                    );
                    break effects;
                }
                Err(e) => {
                    error!("Failed to execute transaction: {:?}", e);
                    coin = self
                        .sui_client
                        .get_latest_gas_objects([coin.object_ref.0])
                        .await
                        .into_iter()
                        .next()
                        .unwrap()
                        .1
                        .unwrap();
                    continue;
                }
            }
        };
        let mut result = vec![];
        let new_coin_balance = (coin.balance - budget) / split_count;
        for created in effects.created() {
            result.extend(self.enqueue_task(GasCoin {
                object_ref: created.reference.to_object_ref(),
                balance: new_coin_balance,
            }));
        }
        let remaining_coin_balance = (coin.balance - new_coin_balance * (split_count - 1)) as i64
            - effects.gas_cost_summary().net_gas_usage();
        result.extend(self.enqueue_task(GasCoin {
            object_ref: effects.gas_object().reference.to_object_ref(),
            balance: remaining_coin_balance as u64,
        }));
        self.increment_total_coin_count_by(result.len() - 1);
        result
    }
}

pub struct GasPoolInitializer {
    _task_handle: JoinHandle<()>,
    // This is always Some. It is None only after the drop method is called.
    cancel_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for GasPoolInitializer {
    fn drop(&mut self) {
        self.cancel_sender.take().unwrap().send(()).unwrap();
    }
}

impl GasPoolInitializer {
    pub async fn start(
        fullnode_url: String,
        storage: Arc<dyn Storage>,
        coin_init_config: CoinInitConfig,
        signer: Arc<dyn TxSigner>,
    ) -> Self {
        // Always run once at the beginning to make sure we have enough coins.
        Self::run_once(fullnode_url.as_str(), &storage, &coin_init_config, &signer).await;
        let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel();
        let _task_handle = tokio::spawn(Self::run(
            fullnode_url,
            storage,
            coin_init_config,
            signer,
            cancel_receiver,
        ));
        Self {
            _task_handle,
            cancel_sender: Some(cancel_sender),
        }
    }

    async fn run(
        fullnode_url: String,
        storage: Arc<dyn Storage>,
        coin_init_config: CoinInitConfig,
        signer: Arc<dyn TxSigner>,
        mut cancel_receiver: tokio::sync::oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(coin_init_config.refresh_interval_sec)) => {}
                _ = &mut cancel_receiver => {
                    info!("Coin init task is cancelled");
                    break;
                }
            }
            info!("Coin init task waking up and looking for new coins to initialize");
            Self::run_once(fullnode_url.as_str(), &storage, &coin_init_config, &signer).await;
        }
    }

    async fn run_once(
        fullnode_url: &str,
        storage: &Arc<dyn Storage>,
        coin_init_config: &CoinInitConfig,
        signer: &Arc<dyn TxSigner>,
    ) {
        let start = Instant::now();
        let sponsor_address = signer.get_address();
        // If we have never initialized the gas pool, we must at least do it once.
        let force_init = !storage.is_initialized(sponsor_address).await.unwrap();
        let sui_client = SuiClient::new(fullnode_url).await;
        let balance_threshold = if force_init {
            0
        } else {
            coin_init_config.target_init_balance * NEW_COIN_BALANCE_FACTOR_THRESHOLD
        };
        let coins = sui_client
            .get_all_owned_sui_coins_above_balance_threshold(sponsor_address, balance_threshold)
            .await;
        if coins.is_empty() {
            info!(
                "No coins with balance above {} found. Skipping new coin initialization",
                balance_threshold
            );
            return;
        }
        let total_coin_count = Arc::new(AtomicUsize::new(coins.len()));
        let rgp = sui_client.get_reference_gas_price().await;
        let gas_cost_per_object = sui_client
            .calibrate_gas_cost_per_object(sponsor_address, &coins[0])
            .await;
        info!("Calibrated gas cost per object: {:?}", gas_cost_per_object);
        let result = Self::split_gas_coins(
            coins,
            CoinSplitEnv {
                target_init_coin_balance: coin_init_config.target_init_balance,
                gas_cost_per_object,
                signer: signer.clone(),
                sui_client,
                task_queue: Default::default(),
                total_coin_count,
                rgp,
            },
        )
        .await;
        for chunk in result.chunks(5000) {
            storage
                .add_new_coins(sponsor_address, chunk.to_vec())
                .await
                .unwrap();
        }
        info!(
            "New coin initialization took {:?}s",
            start.elapsed().as_secs()
        );
    }

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
        let new_total_balance: u64 = result.iter().map(|c| c.balance).sum();
        info!(
            "Splitting finished. Got {} coins. New total balance: {}. Spent {} gas in total",
            result.len(),
            new_total_balance,
            total_balance - new_total_balance
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::config::CoinInitConfig;
    use crate::gas_pool_initializer::{GasPoolInitializer, NEW_COIN_BALANCE_FACTOR_THRESHOLD};
    use crate::storage::connect_storage_for_testing;
    use crate::test_env::start_sui_cluster;
    use sui_types::gas_coin::MIST_PER_SUI;

    // TODO: Add more accurate tests.

    #[tokio::test]
    async fn test_basic_init_flow() {
        telemetry_subscribers::init_for_testing();
        let (cluster, signer) = start_sui_cluster(vec![1000 * MIST_PER_SUI]).await;
        let sponsor = signer.get_address();
        let fullnode_url = cluster.fullnode_handle.rpc_url;
        let storage = connect_storage_for_testing().await;
        let _ = GasPoolInitializer::start(
            fullnode_url,
            storage.clone(),
            CoinInitConfig {
                target_init_balance: MIST_PER_SUI,
                refresh_interval_sec: 200,
            },
            signer,
        )
        .await;
        assert!(storage.get_available_coin_count(sponsor).await.unwrap() > 900);
    }

    #[tokio::test]
    async fn test_init_non_even_split() {
        telemetry_subscribers::init_for_testing();
        let (cluster, signer) = start_sui_cluster(vec![10000000 * MIST_PER_SUI]).await;
        let sponsor = signer.get_address();
        let fullnode_url = cluster.fullnode_handle.rpc_url;
        let storage = connect_storage_for_testing().await;
        let target_init_balance = 12345 * MIST_PER_SUI;
        let _ = GasPoolInitializer::start(
            fullnode_url,
            storage.clone(),
            CoinInitConfig {
                target_init_balance,
                refresh_interval_sec: 200,
            },
            signer,
        )
        .await;
        assert!(storage.get_available_coin_count(sponsor).await.unwrap() > 800);
    }

    #[tokio::test]
    async fn test_add_new_funds_to_pool() {
        telemetry_subscribers::init_for_testing();
        let (cluster, signer) = start_sui_cluster(vec![1000 * MIST_PER_SUI]).await;
        let sponsor = signer.get_address();
        let fullnode_url = cluster.fullnode_handle.rpc_url.clone();
        let storage = connect_storage_for_testing().await;
        let _init_task = GasPoolInitializer::start(
            fullnode_url,
            storage.clone(),
            CoinInitConfig {
                target_init_balance: MIST_PER_SUI,
                refresh_interval_sec: 1,
            },
            signer,
        )
        .await;
        assert!(storage.is_initialized(sponsor).await.unwrap());
        let available_coin_count = storage.get_available_coin_count(sponsor).await.unwrap();

        // Transfer some new SUI into the sponsor account.
        let new_addr = *cluster
            .get_addresses()
            .iter()
            .find(|addr| **addr != sponsor)
            .unwrap();
        let tx_data = cluster
            .test_transaction_builder_with_sender(new_addr)
            .await
            .transfer_sui(
                Some(NEW_COIN_BALANCE_FACTOR_THRESHOLD * MIST_PER_SUI),
                sponsor,
            )
            .build();
        cluster.sign_and_execute_transaction(&tx_data).await;
        // Give it some time for the task to pick up the new coin and split it.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let new_available_coin_count = storage.get_available_coin_count(sponsor).await.unwrap();
        assert!(new_available_coin_count > available_coin_count + 100);
    }
}
