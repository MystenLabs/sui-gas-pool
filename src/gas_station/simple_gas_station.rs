// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_station::GasStation;
use crate::storage::Storage;
use crate::types::{GasCoin, GaslessTransaction};
use anyhow::bail;
use parking_lot::Mutex;
use shared_crypto::intent::{Intent, IntentMessage};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sui_json_rpc_types::{SuiData, SuiObjectDataOptions, SuiObjectResponse};
use sui_sdk::{SuiClient, SuiClientBuilder};
use sui_types::base_types::{ObjectID, SuiAddress};
use sui_types::crypto::{AccountKeyPair, Signature};
use sui_types::signature::GenericSignature;
use sui_types::transaction::{GasData, TransactionData, TransactionDataV1, TransactionKind};
use tracing::debug;

// TODO: Implement timer based gas coin unlocker.

pub struct SimpleGasStation {
    sponsor: SuiAddress,
    keypair: Arc<AccountKeyPair>,
    gas_pool_store: Arc<dyn Storage>,
    sui_client: Arc<SuiClient>,
    locked_gas_coins: Arc<Mutex<HashSet<ObjectID>>>,
    unlock_queue: Arc<Mutex<BinaryHeap<CoinLockInfo>>>,
}

#[derive(Copy, Clone, Debug, Eq)]
struct CoinLockInfo {
    gas_object: ObjectID,
    unlock_time: Instant,
}

impl PartialOrd<Self> for CoinLockInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(other.cmp(self))
    }
}

impl Ord for CoinLockInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        other.unlock_time.cmp(&self.unlock_time)
    }
}

impl PartialEq for CoinLockInfo {
    fn eq(&self, other: &Self) -> bool {
        self.unlock_time == other.unlock_time
    }
}

impl SimpleGasStation {
    pub async fn new(
        sponsor: SuiAddress,
        keypair: AccountKeyPair,
        gas_pool_store: Arc<dyn Storage>,
        fullnode_url: &str,
    ) -> Self {
        let sui_client = Arc::new(
            SuiClientBuilder::default()
                .build(fullnode_url)
                .await
                .unwrap(),
        );
        Self {
            sponsor,
            keypair: Arc::new(keypair),
            gas_pool_store,
            sui_client,
            unlock_queue: Arc::new(Mutex::new(BinaryHeap::new())),
            locked_gas_coins: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn try_get_sui_coin_balance(object: &SuiObjectResponse) -> Option<GasCoin> {
        let data = object.data.as_ref()?;
        let object_ref = data.object_ref();
        let move_obj = data.bcs.as_ref()?.try_as_move()?;
        if &move_obj.type_ != &sui_types::gas_coin::GasCoin::type_() {
            return None;
        }
        let gas_coin: sui_types::gas_coin::GasCoin = bcs::from_bytes(&move_obj.bcs_bytes).ok()?;
        Some(GasCoin {
            object_ref,
            balance: gas_coin.value(),
        })
    }

    async fn multi_get_gas_balance(&self, coins: &[ObjectID]) -> Vec<GasCoin> {
        let objects = tokio_retry::Retry::spawn(
            tokio_retry::strategy::ExponentialBackoff::from_millis(10)
                .max_delay(Duration::from_secs(600)),
            || async {
                let objects = self
                    .sui_client
                    .read_api()
                    .multi_get_object_with_options(
                        coins.to_vec(),
                        SuiObjectDataOptions::default().with_bcs(),
                    )
                    .await
                    .map_err(|e| anyhow::Error::from(e))?;
                if objects.len() != coins.len() {
                    bail!(
                        "Unable to get all gas coins, got {} out of {}",
                        objects.len(),
                        coins.len()
                    );
                }
                Ok(objects)
            },
        )
        .await
        .expect("Unable to connect to fullnode after retries, exiting");
        objects
            .iter()
            .zip(coins)
            .filter_map(|(o, id)| match Self::try_get_sui_coin_balance(o) {
                Some(coin) => {
                    debug!("Got updated gas coin info {:?}", coin);
                    Some(coin)
                }
                None => {
                    debug!("Unable to get gas coin info for object {:?}", id);
                    None
                }
            })
            .collect()
    }

    async fn release_gas_impl(&self, coins: Vec<ObjectID>) -> usize {
        let gas_coins = self.multi_get_gas_balance(&coins).await;
        let len = gas_coins.len();
        self.gas_pool_store.add_gas_coins(gas_coins);
        {
            let mut guard = self.locked_gas_coins.lock();
            for id in &coins {
                guard.remove(id);
            }
        }
        len
    }
}

#[async_trait::async_trait]
impl GasStation for SimpleGasStation {
    async fn reserve_gas(
        &self,
        tx: GaslessTransaction,
        duration: Duration,
    ) -> anyhow::Result<(IntentMessage<TransactionData>, GenericSignature)> {
        let gas_coins = self.gas_pool_store.reserve_gas_coins(tx.budget);
        if gas_coins.is_empty() {
            bail!("Unable to find enough gas coins");
        }
        self.locked_gas_coins
            .lock()
            .extend(gas_coins.iter().map(|c| c.object_ref.0));
        let unlock_time = Instant::now().add(duration);
        self.unlock_queue
            .lock()
            .extend(gas_coins.iter().map(|c| CoinLockInfo {
                gas_object: c.object_ref.0,
                unlock_time,
            }));
        let tx_data = TransactionData::V1(TransactionDataV1 {
            kind: TransactionKind::ProgrammableTransaction(tx.pt),
            sender: tx.sender,
            gas_data: GasData {
                payment: gas_coins.into_iter().map(|c| c.object_ref).collect(),
                owner: self.sponsor,
                price: tx.price,
                budget: tx.budget,
            },
            expiration: tx.expiration,
        });
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data);
        let sig = Signature::new_secure(&intent_msg, self.keypair.as_ref());
        Ok((intent_msg, sig.into()))
    }

    async fn release_gas(&self, coins: Vec<ObjectID>) -> usize {
        self.release_gas_impl(coins).await
    }
}
