// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::metrics::GasPoolCoreMetrics;
use crate::retry_forever;
use crate::storage::Storage;
use crate::sui_client::SuiClient;
use crate::types::ReservationID;
use anyhow::bail;
use shared_crypto::intent::{Intent, IntentMessage};
use std::collections::HashMap;
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
    metrics: Arc<GasPoolCoreMetrics>,
}

impl GasPool {
    pub async fn new(
        keypairs: HashMap<SuiAddress, Arc<SuiKeyPair>>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
        metrics: Arc<GasPoolCoreMetrics>,
    ) -> Arc<Self> {
        let pool = Arc::new(Self {
            keypairs,
            gas_pool_store,
            sui_client: SuiClient::new(fullnode_url).await,
            metrics,
        });
        for address in pool.keypairs.keys() {
            let available_coin_count = pool.query_pool_available_coin_count(*address).await;
            pool.metrics
                .gas_pool_available_gas_coin_count
                .with_label_values(&[&address.to_string()])
                .set(available_coin_count as i64);
        }
        pool
    }

    pub async fn reserve_gas(
        &self,
        request_sponsor: Option<SuiAddress>,
        gas_budget: u64,
        duration: Duration,
    ) -> anyhow::Result<(SuiAddress, ReservationID, Vec<ObjectRef>)> {
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
        let (reservation_id, gas_coins) = self
            .gas_pool_store
            .reserve_gas_coins(sponsor, gas_budget, duration.as_millis() as u64)
            .await?;
        info!(
            "Reserved gas coins with sponsor={:?}, budget={:?} and duration={:?}: {:?}",
            sponsor, gas_budget, duration, gas_coins
        );
        self.metrics
            .reserved_gas_coin_count_per_request
            .observe(gas_coins.len() as u64);
        self.metrics
            .gas_pool_available_gas_coin_count
            .with_label_values(&[&sponsor.to_string()])
            .sub(gas_coins.len() as i64);
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
        let keypair = match self.keypairs.get(&sponsor) {
            Some(keypair) => keypair.as_ref(),
            None => bail!("Sponsor {:?} is not registered", sponsor),
        };
        let payment: Vec<_> = tx_data
            .gas_data()
            .payment
            .iter()
            .map(|oref| oref.0)
            .collect();
        debug!(
            ?reservation_id,
            "Payment coins in transaction: {:?}", payment
        );
        self.gas_pool_store
            .ready_for_execution(sponsor, reservation_id)
            .await?;
        debug!(?reservation_id, "Reservation is ready for execution");

        let intent_msg = IntentMessage::new(Intent::sui_transaction(), &tx_data);
        let sponsor_sig = Signature::new_secure(&intent_msg, keypair);
        let tx = Transaction::from_generic_sig_data(tx_data, vec![sponsor_sig.into(), user_sig]);
        let response = self
            .sui_client
            .execute_transaction(tx, Duration::from_secs(60))
            .await;
        // Regardless of whether the transaction succeeded, we need to release the coins.
        self.release_gas_coins(sponsor, payment).await;
        response
    }

    async fn release_gas_coins(&self, sponsor_address: SuiAddress, gas_coins: Vec<ObjectID>) {
        debug!(
            "Trying to release gas coins. Sponsor: {:?}, coins: {:?}",
            sponsor_address, gas_coins
        );

        let latest = self.sui_client.get_latest_gas_objects(gas_coins).await;
        debug!("Latest coin state: {:?}", latest);
        let updated_coins: Vec<_> = latest.into_values().flatten().collect();

        retry_forever!(async {
            self.gas_pool_store
                .add_new_coins(sponsor_address, updated_coins.clone())
                .await
                .tap_err(|err| error!("Failed to call update_gas_coins on storage: {:?}", err))
        })
        .unwrap();
        self.metrics
            .gas_pool_available_gas_coin_count
            .with_label_values(&[&sponsor_address.to_string()])
            .add(updated_coins.len() as i64);
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
                        self.release_gas_coins(*address, unlocked_coins).await;
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

    pub async fn query_pool_available_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        self.gas_pool_store
            .get_available_coin_count(sponsor_address)
            .await
    }
}

impl GasPoolContainer {
    pub async fn new(
        keypair: Arc<SuiKeyPair>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
        run_coin_expiring_task: bool,
        metrics: Arc<GasPoolCoreMetrics>,
    ) -> Self {
        let sponsor = (&keypair.public()).into();
        let keypairs = HashMap::from([(sponsor, keypair)]);
        let inner = GasPool::new(keypairs, gas_pool_store, fullnode_url, metrics).await;
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
