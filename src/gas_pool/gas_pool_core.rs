// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::metrics::GasPoolMetrics;
use crate::retry_forever;
use crate::storage::Storage;
use crate::sui_client::SuiClient;
use crate::types::UpdatedGasGroup;
use anyhow::bail;
use shared_crypto::intent::{Intent, IntentMessage};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;
use sui_json_rpc_types::SuiTransactionBlockEffects;
use sui_types::base_types::{ObjectID, ObjectRef, SuiAddress};
use sui_types::crypto::{Signature, SuiKeyPair};
use sui_types::signature::GenericSignature;
use sui_types::transaction::{Transaction, TransactionData, TransactionDataAPI};
use tap::TapFallible;
use tokio::task::JoinHandle;
#[cfg(not(test))]
use tokio_retry::strategy::FixedInterval;
#[cfg(not(test))]
use tokio_retry::Retry;
use tracing::{debug, error, info};

// TODO: Figure out the right max duration.
// 10 mins.
const MAX_DURATION: Duration = Duration::from_secs(10 * 60);

// TODO: Add crash recovery using a persistent storage.

pub struct GasPoolContainer {
    inner: Arc<GasPool>,
    _coin_unlocker_task: Option<JoinHandle<()>>,
    cancel_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct GasPool {
    keypairs: HashMap<SuiAddress, Arc<SuiKeyPair>>,
    gas_pool_store: Arc<dyn Storage>,
    sui_client: SuiClient,
    metrics: Arc<GasPoolMetrics>,
}

impl GasPool {
    pub async fn reserve_gas(
        &self,
        request_sponsor: Option<SuiAddress>,
        gas_budget: u64,
        duration: Duration,
    ) -> anyhow::Result<(SuiAddress, Vec<ObjectRef>)> {
        let sponsor = match request_sponsor {
            Some(sponsor) => {
                if !self.keypairs.contains_key(&sponsor) {
                    bail!("Sponsor {:?} is not registered", sponsor);
                };
                sponsor
            }
            // unwrap is safe because the gas station is constructed using some keypair.
            None => *self.keypairs.keys().next().unwrap(),
        };
        if duration > MAX_DURATION {
            bail!(
                "Duration {:?} is longer than the maximum allowed duration {:?}",
                duration,
                MAX_DURATION
            );
        }
        let gas_coins = self
            .gas_pool_store
            .reserve_gas_coins(sponsor, gas_budget, duration.as_millis() as u64)
            .await
            .tap_err(|_| {
                self.metrics.num_failed_storage_pool_reservation.inc();
            })?;
        info!(
            "Reserved gas coins with sponsor={:?}, budget={:?} and duration={:?}: {:?}",
            sponsor, gas_budget, duration, gas_coins
        );
        self.metrics.num_successful_storage_pool_reservation.inc();
        self.metrics.cur_num_alive_reservations.inc();
        self.metrics
            .cur_num_reserved_gas_coins
            .add(gas_coins.len() as i64);
        Ok((
            sponsor,
            gas_coins.into_iter().map(|c| c.object_ref).collect(),
        ))
    }

    pub async fn execute_transaction(
        &self,
        tx_data: TransactionData,
        user_sig: GenericSignature,
    ) -> anyhow::Result<SuiTransactionBlockEffects> {
        let sponsor = tx_data.gas_data().owner;
        let keypair = match self.keypairs.get(&sponsor) {
            Some(keypair) => keypair.as_ref(),
            None => bail!("Sponsor {:?} is not registered", sponsor),
        };
        let payment: BTreeSet<ObjectID> = tx_data
            .gas_data()
            .payment
            .iter()
            .map(|oref| oref.0)
            .collect();
        debug!("Payment coins in transaction: {:?}", payment);
        self.gas_pool_store
            .read_for_execution(sponsor, payment.clone())
            .await?;
        self.metrics.num_released_reservations.inc();
        self.metrics
            .num_released_gas_coins
            .inc_by(payment.len() as u64);

        let intent_msg = IntentMessage::new(Intent::sui_transaction(), &tx_data);
        let sponsor_sig = Signature::new_secure(&intent_msg, keypair);
        let tx = Transaction::from_generic_sig_data(tx_data, vec![sponsor_sig.into(), user_sig]);
        let response = self
            .sui_client
            .execute_transaction(tx, Duration::from_secs(60))
            .await;
        // Regardless of whether the transaction succeeded, we need to release the coins.
        self.release_gas_coins(sponsor, &[payment]).await;
        self.metrics.num_released_reservations.inc();
        response
    }

    async fn release_gas_coins(
        &self,
        sponsor_address: SuiAddress,
        gas_coin_groups: &[BTreeSet<ObjectID>],
    ) {
        debug!(
            "Trying to release gas coins. Sponsor: {:?}, coins: {:?}",
            sponsor_address, gas_coin_groups
        );
        let all_object_ids = gas_coin_groups.iter().flatten().cloned();

        let latest = self.sui_client.get_latest_gas_objects(all_object_ids).await;
        debug!("Latest coin state: {:?}", latest);
        let updated_groups: Vec<_> = gas_coin_groups
            .into_iter()
            .map(|group| {
                let updated_gas_coins: Vec<_> = group
                    .iter()
                    .filter_map(|id| latest.get(id).unwrap().clone())
                    .collect();
                let deleted_gas_coins: Vec<_> = group
                    .iter()
                    .filter(|id| latest.get(id).unwrap().is_none())
                    .cloned()
                    .collect();
                UpdatedGasGroup::new(updated_gas_coins, deleted_gas_coins)
            })
            .collect();

        retry_forever!(async {
            self.gas_pool_store
                .update_gas_coins(sponsor_address, updated_groups.clone())
                .await
                .tap_err(|err| error!("Failed to call update_gas_coins on storage: {:?}", err))
        })
        .unwrap();
    }

    async fn start_coin_unlock_task(
        self: Arc<Self>,
        mut cancel_receiver: tokio::sync::oneshot::Receiver<()>,
    ) -> JoinHandle<()> {
        tokio::task::spawn(async move {
            loop {
                for address in self.keypairs.keys() {
                    let expire_results = self.gas_pool_store.expire_coins(*address).await;
                    let unlocked_coins = expire_results.unwrap_or_else(|err| {
                        error!("Failed to call expire_coins to the storage: {:?}", err);
                        vec![]
                    });
                    if !unlocked_coins.is_empty() {
                        debug!("Coins that are expired: {:?}", unlocked_coins);
                        self.release_gas_coins(*address, &unlocked_coins).await;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    _ = &mut cancel_receiver => {
                        info!("Coin unlocker task is cancelled");
                        break;
                    }
                }
            }
        })
    }

    #[cfg(test)]
    pub async fn query_pool_available_coin_count(&self) -> usize {
        self.gas_pool_store.get_available_coin_count().await
    }
}

impl GasPoolContainer {
    pub async fn new(
        keypair: Arc<SuiKeyPair>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
        run_coin_expiring_task: bool,
        metrics: Arc<GasPoolMetrics>,
    ) -> Self {
        let sui_client = SuiClient::new(fullnode_url).await;
        let sponsor = (&keypair.public()).into();
        let keypairs = HashMap::from([(sponsor, keypair)]);
        let inner = Arc::new(GasPool {
            keypairs,
            gas_pool_store,
            sui_client,
            metrics,
        });
        let (_coin_unlocker_task, cancel_sender) = if run_coin_expiring_task {
            let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel();
            let coin_unlocker_task = inner.clone().start_coin_unlock_task(cancel_receiver).await;
            (Some(coin_unlocker_task), Some(cancel_sender))
        } else {
            (None, None)
        };

        Self {
            inner,
            _coin_unlocker_task,
            cancel_sender,
        }
    }

    pub fn get_gas_pool_arc(&self) -> Arc<GasPool> {
        self.inner.clone()
    }
}

impl Drop for GasPoolContainer {
    fn drop(&mut self) {
        if let Some(sender) = self.cancel_sender.take() {
            sender.send(()).unwrap();
        }
    }
}
