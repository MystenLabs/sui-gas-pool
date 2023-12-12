// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_station::locked_gas_coins::LockedGasCoins;
use crate::retry_forever;
use crate::storage::Storage;
use crate::sui_client::SuiClient;
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
use tokio::task::JoinHandle;
#[cfg(not(test))]
use tokio_retry::strategy::FixedInterval;
#[cfg(not(test))]
use tokio_retry::Retry;
use tracing::{debug, info};

// TODO: Add crash recovery using a persistent storage.

pub struct GasStationContainer {
    inner: Arc<GasStation>,
    _coin_unlocker_task: JoinHandle<()>,
    cancel_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct GasStation {
    keypairs: HashMap<SuiAddress, Arc<SuiKeyPair>>,
    gas_pool_store: Arc<dyn Storage>,
    sui_client: SuiClient,
    locked_gas_coins: LockedGasCoins,
}

impl GasStation {
    async fn start_coin_unlock_task(
        self: Arc<Self>,
        mut cancel_receiver: tokio::sync::oneshot::Receiver<()>,
    ) -> JoinHandle<()> {
        tokio::task::spawn(async move {
            loop {
                let unlocked_coins = self.locked_gas_coins.unlock_if_expired();
                if !unlocked_coins.is_empty() {
                    let mut unlocked_coins_map: HashMap<SuiAddress, Vec<ObjectID>> = HashMap::new();
                    for lock_info in unlocked_coins {
                        unlocked_coins_map
                            .entry(lock_info.inner.sponsor)
                            .or_default()
                            .extend(lock_info.inner.objects.clone());
                    }
                    for (sponsor, gas_coins) in unlocked_coins_map {
                        // Break into chunks to avoid hitting RPC limits.
                        for chunk in gas_coins.chunks(2000) {
                            self.release_gas_coins(sponsor, chunk).await;
                        }
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

    async fn release_gas_coins(&self, sponsor_address: SuiAddress, gas_coins: &[ObjectID]) {
        debug!(
            "Trying to release gas coins. Sponsor: {:?}, coins: {:?}",
            sponsor_address, gas_coins
        );
        let latest = self.sui_client.get_latest_gas_objects(gas_coins).await;
        retry_forever!(async {
            self.gas_pool_store
                .update_gas_coins(
                    sponsor_address,
                    latest.live_gas_coins.clone(),
                    latest.deleted_gas_coins.clone(),
                )
                .await
        })
        .unwrap();
        debug!(
            "Released {} coins to back to the pool, and deleted {} coins permanently",
            latest.live_gas_coins.len(),
            latest.deleted_gas_coins.len()
        );
    }

    pub async fn reserve_gas(
        &self,
        request_sponsor: Option<SuiAddress>,
        gas_budget: u64,
        duration: Duration,
    ) -> anyhow::Result<(SuiAddress, Vec<ObjectRef>)> {
        let sponsor =
            match request_sponsor {
                Some(sponsor) => {
                    if !self.keypairs.contains_key(&sponsor) {
                        bail!("Sponsor {:?} is not registered", sponsor);
                    };
                    sponsor
                }
                None => *self.keypairs.keys().next().ok_or_else(|| {
                    anyhow::anyhow!("No sponsor is registered in the gas station")
                })?,
            };
        let gas_coins = self
            .gas_pool_store
            .reserve_gas_coins(sponsor, gas_budget)
            .await?;
        self.locked_gas_coins
            .add_locked_coins(sponsor, &gas_coins, duration);
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
        let payment: Vec<ObjectID> = tx_data
            .gas_data()
            .payment
            .iter()
            .map(|oref| oref.0)
            .collect();
        debug!("Payment coins: {:?}", payment);
        self.locked_gas_coins.remove_locked_coins(&payment)?;

        // TODO: Remove clone once we have a better Transaction construction API.
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data.clone());
        let sponsor_sig = Signature::new_secure(&intent_msg, keypair);
        let tx = Transaction::from_generic_sig_data(
            tx_data,
            Intent::sui_transaction(),
            vec![sponsor_sig.into(), user_sig],
        );
        let response = self
            .sui_client
            .execute_transaction(tx, Duration::from_secs(60))
            .await;
        // Regardless of whether the transaction succeeded, we need to release the coins.
        self.release_gas_coins(sponsor, &payment).await;
        response
    }

    #[cfg(test)]
    pub async fn get_available_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        self.gas_pool_store
            .get_available_coin_count(sponsor_address)
            .await
    }
}

impl GasStationContainer {
    pub async fn new(
        keypair: Arc<SuiKeyPair>,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
    ) -> Self {
        let sui_client = SuiClient::new(fullnode_url).await;
        let sponsor = (&keypair.public()).into();
        let keypairs = HashMap::from([(sponsor, keypair)]);
        let inner = Arc::new(GasStation {
            keypairs,
            gas_pool_store,
            sui_client,
            locked_gas_coins: LockedGasCoins::default(),
        });
        let (cancel_sender, cancel_receiver) = tokio::sync::oneshot::channel();
        let _coin_unlocker_task = inner.clone().start_coin_unlock_task(cancel_receiver).await;
        Self {
            inner,
            _coin_unlocker_task,
            cancel_sender: Some(cancel_sender),
        }
    }

    pub fn get_station(&self) -> Arc<GasStation> {
        self.inner.clone()
    }
}

impl Drop for GasStationContainer {
    fn drop(&mut self) {
        self.cancel_sender.take().unwrap().send(()).unwrap();
    }
}
