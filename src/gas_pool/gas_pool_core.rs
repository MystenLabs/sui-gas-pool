// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::metrics::GasPoolCoreMetrics;
use crate::storage::Storage;
use crate::sui_client::SuiClient;
use crate::tx_signer::TxSigner;
use crate::types::ReservationID;
use crate::{retry_forever, retry_with_max_delay};
use anyhow::bail;
use std::sync::Arc;
use std::time::Duration;
use sui_json_rpc_types::{SuiTransactionBlockEffects, SuiTransactionBlockEffectsAPI};
use sui_types::base_types::{ObjectID, ObjectRef, SuiAddress};
use sui_types::signature::GenericSignature;
use sui_types::transaction::{Argument, Command, Transaction, TransactionData, TransactionDataAPI};
use tap::TapFallible;
use tokio::task::JoinHandle;
use tokio_retry::strategy::ExponentialBackoff;
#[cfg(not(test))]
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::{debug, error, info};

use super::gas_usage_cap::GasUsageCap;

const EXPIRATION_JOB_INTERVAL: Duration = Duration::from_secs(1);

pub struct GasPoolContainer {
    inner: Arc<GasPool>,
    _coin_unlocker_task: JoinHandle<()>,
    // This is always Some. It is None only after the drop method is called.
    cancel_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct GasPool {
    signer: Arc<dyn TxSigner>,
    gas_pool_store: Arc<dyn Storage>,
    sui_client: SuiClient,
    metrics: Arc<GasPoolCoreMetrics>,
    gas_usage_cap: Arc<GasUsageCap>,
}

impl GasPool {
    pub async fn new(
        signer: Arc<dyn TxSigner>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
        metrics: Arc<GasPoolCoreMetrics>,
        gas_usage_cap: Arc<GasUsageCap>,
    ) -> Arc<Self> {
        let pool = Self {
            signer,
            gas_pool_store,
            sui_client: SuiClient::new(fullnode_url).await,
            metrics,
            gas_usage_cap,
        };
        Arc::new(pool)
    }

    pub async fn reserve_gas(
        &self,
        gas_budget: u64,
        duration: Duration,
    ) -> anyhow::Result<(SuiAddress, ReservationID, Vec<ObjectRef>)> {
        self.gas_usage_cap.check_usage().await?;
        let sponsor = self.signer.get_address();
        let (reservation_id, gas_coins) = self
            .gas_pool_store
            .reserve_gas_coins(gas_budget, duration.as_millis() as u64)
            .await?;
        self.metrics
            .reserved_gas_coin_count_per_request
            .observe(gas_coins.len() as u64);
        Ok((
            sponsor,
            reservation_id,
            gas_coins.into_iter().map(|c| c.object_ref).collect(),
        ))
    }

    pub async fn execute_transaction(
        &self,
        reservation_id: ReservationID,
        tx_data: TransactionData,
        user_sig: GenericSignature,
    ) -> anyhow::Result<SuiTransactionBlockEffects> {
        let sponsor = tx_data.gas_data().owner;
        if !self.signer.is_valid_address(&sponsor) {
            bail!("Sponsor {:?} is not registered", sponsor);
        };
        Self::check_transaction_validity(&tx_data)?;
        let payment: Vec<_> = tx_data
            .gas_data()
            .payment
            .iter()
            .map(|oref| oref.0)
            .collect();
        let payment_count = payment.len();
        debug!(
            ?reservation_id,
            "Payment coins in transaction: {:?}", payment
        );
        self.gas_pool_store
            .ready_for_execution(reservation_id)
            .await?;
        debug!(?reservation_id, "Reservation is ready for execution");

        let sponsor_sig = retry_with_max_delay!(
            async {
                self.signer
                    .sign_transaction(&tx_data)
                    .await
                    .tap_err(|err| error!("Failed to sign transaction: {:?}", err))
            },
            Duration::from_secs(5)
        )?;
        let tx = Transaction::from_generic_sig_data(tx_data, vec![sponsor_sig, user_sig]);
        let cur_time = std::time::Instant::now();
        let response = self
            .sui_client
            .execute_transaction(tx, Duration::from_secs(60))
            .await;
        let elapsed = cur_time.elapsed().as_millis();
        self.metrics
            .transaction_execution_latency_ms
            .observe(elapsed as u64);

        // Regardless of whether the transaction succeeded, we need to release the coins.
        let release_count = self.release_gas_coins(payment).await;
        if payment_count > release_count {
            let smashed_coin_count = payment_count - release_count;
            info!(
                ?reservation_id,
                "Smashed {:?} coins after transaction execution", smashed_coin_count
            );
            self.metrics
                .num_smashed_gas_coins
                .with_label_values(&[&sponsor.to_string()])
                .inc_by(smashed_coin_count as u64);
        }
        info!(
            ?reservation_id,
            "Released {:?} coins after transaction execution", release_count
        );
        if let Ok(effects) = &response {
            let net_gas_usage = effects.gas_cost_summary().net_gas_usage();
            let new_daily_usage = self.gas_usage_cap.update_usage(net_gas_usage).await;
            self.metrics
                .daily_gas_usage
                .with_label_values(&[&sponsor.to_string()])
                .set(new_daily_usage);
        }
        response
    }

    fn check_transaction_validity(tx_data: &TransactionData) -> anyhow::Result<()> {
        let mut all_args = vec![];
        for command in tx_data.kind().iter_commands() {
            match command {
                Command::MoveCall(call) => {
                    all_args.extend(call.arguments.iter());
                }
                Command::TransferObjects(args, _) => {
                    all_args.extend(args.iter());
                }
                Command::SplitCoins(arg, _) => {
                    all_args.push(arg);
                }
                Command::MergeCoins(arg, args) => {
                    all_args.push(arg);
                    all_args.extend(args.iter());
                }
                Command::Publish(_, _) => {}
                Command::MakeMoveVec(_, args) => {
                    all_args.extend(args.iter());
                }
                Command::Upgrade(_, _, _, _) => {}
            };
        }
        let uses_gas = all_args
            .into_iter()
            .any(|arg| matches!(*arg, Argument::GasCoin));
        if uses_gas {
            bail!("Gas coin can only be used to pay gas")
        };
        Ok(())
    }

    /// Returns number of coins added back to the pool.
    async fn release_gas_coins(&self, gas_coins: Vec<ObjectID>) -> usize {
        debug!("Trying to release gas coins: {:?}", gas_coins);

        let latest = self.sui_client.get_latest_gas_objects(gas_coins).await;
        debug!("Latest coin state: {:?}", latest);
        let updated_coins: Vec<_> = latest.into_values().flatten().collect();

        retry_forever!(async {
            self.gas_pool_store
                .add_new_coins(updated_coins.clone())
                .await
                .tap_err(|err| error!("Failed to call update_gas_coins on storage: {:?}", err))
        })
        .unwrap();
        updated_coins.len()
    }

    async fn start_coin_unlock_task(
        self: Arc<Self>,
        mut cancel_receiver: tokio::sync::oneshot::Receiver<()>,
    ) -> JoinHandle<()> {
        tokio::task::spawn(async move {
            loop {
                let expire_results = self.gas_pool_store.expire_coins().await;
                let unlocked_coins = expire_results.unwrap_or_else(|err| {
                    error!("Failed to call expire_coins to the storage: {:?}", err);
                    vec![]
                });
                if !unlocked_coins.is_empty() {
                    debug!("Coins that are expired: {:?}", unlocked_coins);
                    let count = self.release_gas_coins(unlocked_coins).await;
                    info!("Released {:?} coins after expiration", count);
                }
                tokio::select! {
                    _ = tokio::time::sleep(EXPIRATION_JOB_INTERVAL) => {}
                    _ = &mut cancel_receiver => {
                        info!("Coin unlocker task is cancelled");
                        break;
                    }
                }
            }
        })
    }

    pub async fn query_pool_available_coin_count(&self) -> usize {
        self.gas_pool_store
            .get_available_coin_count()
            .await
            .unwrap()
    }
}

impl GasPoolContainer {
    pub async fn new(
        signer: Arc<dyn TxSigner>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
        gas_usage_daily_cap: u64,
        metrics: Arc<GasPoolCoreMetrics>,
    ) -> Self {
        let inner = GasPool::new(
            signer,
            gas_pool_store,
            fullnode_url,
            metrics,
            Arc::new(GasUsageCap::new(gas_usage_daily_cap)),
        )
        .await;
        let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel();
        let _coin_unlocker_task = inner.clone().start_coin_unlock_task(cancel_receiver).await;

        Self {
            inner,
            _coin_unlocker_task,
            cancel_sender: Some(cancel_sender),
        }
    }

    pub fn get_gas_pool_arc(&self) -> Arc<GasPool> {
        self.inner.clone()
    }
}

impl Drop for GasPoolContainer {
    fn drop(&mut self) {
        self.cancel_sender.take().unwrap().send(()).unwrap();
    }
}
