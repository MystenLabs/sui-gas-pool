// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::types::GasCoin;
use parking_lot::Mutex;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sui_types::base_types::{ObjectID, SuiAddress};
use tracing::debug;

#[derive(Default)]
pub struct LockedGasCoins {
    // TODO: The mutex can be sharded among sponsor addresses.
    mutex: Mutex<LockedGasCoinsInner>,
}

#[derive(Default)]
struct LockedGasCoinsInner {
    /// A lookup map from each ObjectID to the corresponding lock information.
    locked_gas_coins: HashMap<ObjectID, CoinLockInfo>,
    // Use Reverse so that it becomes a min-heap.
    unlock_queue: BinaryHeap<Reverse<CoinLockInfo>>,
}

impl LockedGasCoinsInner {
    pub fn add_locked_coins(
        &mut self,
        sponsor: SuiAddress,
        gas_coins: &[GasCoin],
        lock_duration: Duration,
    ) {
        let unlock_time = Instant::now().add(lock_duration);
        let lock_info = CoinLockInfo::new(
            sponsor,
            gas_coins.iter().map(|c| c.object_ref.0).collect(),
            unlock_time,
        );
        self.locked_gas_coins.extend(
            gas_coins
                .iter()
                .map(|c| (c.object_ref.0, lock_info.clone())),
        );
        self.unlock_queue.push(Reverse(lock_info));
    }

    pub fn unlock_if_expired(&mut self) -> Vec<CoinLockInfo> {
        let now = Instant::now();
        let mut unlocked_coins = Vec::new();
        while let Some(coin_info) = self.unlock_queue.peek() {
            if coin_info.0.inner.unlock_time <= now {
                let coin_info = self.unlock_queue.pop().unwrap().0;
                // If we fail to remove these coins from the locked coins, it means they have already
                // been released proactively.
                // Only return them if we can remove them from the locked coins.
                if self
                    .remove_locked_coins(
                        &coin_info.inner.objects.iter().cloned().collect::<Vec<_>>(),
                    )
                    .is_ok()
                {
                    debug!(
                        "Coins {:?} can be unlocked because its unlock time is before now {:?}",
                        coin_info.inner, now
                    );
                    unlocked_coins.push(coin_info);
                }
            } else {
                break;
            }
        }
        unlocked_coins
    }

    /// If any coin is not currently locked, or they are not all locked by the
    /// same transaction, returns error.
    pub fn remove_locked_coins(&mut self, gas_coins: &[ObjectID]) -> anyhow::Result<()> {
        if gas_coins.is_empty() {
            anyhow::bail!("No gas coin provided");
        }
        let unique_gas_coins = gas_coins.iter().cloned().collect::<HashSet<_>>();
        let mut unique_lock_info = None;
        for c in gas_coins {
            if let Some(lock_info) = self.locked_gas_coins.get(c) {
                if let Some(unique_lock_info) = unique_lock_info {
                    // TODO: Make the comparison more efficient.
                    if unique_lock_info != lock_info {
                        anyhow::bail!("Some gas coins are locked by different transaction")
                    }
                }
                unique_lock_info = Some(lock_info);
            } else {
                anyhow::bail!("Coin {} is not locked", c)
            }
        }
        // unwrap safe because we have checked that gas_coins is not empty,
        // and one iteration will either return early or set unique_lock_info.
        if unique_lock_info.unwrap().inner.objects != unique_gas_coins {
            anyhow::bail!("Gas coins provided are inconsistent with the locked ones");
        }
        for c in gas_coins {
            self.locked_gas_coins.remove(c);
        }
        Ok(())
        // We don't remove them in the unlock queue because it's too expensive to remove
        // by ID from a heap. Eventually they will pop out and be removed anyway.
    }
}

impl LockedGasCoins {
    pub fn add_locked_coins(
        &self,
        sponsor: SuiAddress,
        gas_coins: &[GasCoin],
        lock_duration: Duration,
    ) {
        self.mutex
            .lock()
            .add_locked_coins(sponsor, gas_coins, lock_duration);
    }

    pub fn unlock_if_expired(&self) -> Vec<CoinLockInfo> {
        self.mutex.lock().unlock_if_expired()
    }

    pub fn remove_locked_coins(&self, gas_coins: &[ObjectID]) -> anyhow::Result<()> {
        self.mutex.lock().remove_locked_coins(gas_coins)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoinLockInfo {
    pub inner: Arc<CoinLockInfoInner>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CoinLockInfoInner {
    pub sponsor: SuiAddress,
    pub objects: HashSet<ObjectID>,
    pub unlock_time: Instant,
}

impl CoinLockInfo {
    fn new(sponsor: SuiAddress, objects: HashSet<ObjectID>, unlock_time: Instant) -> Self {
        Self {
            inner: Arc::new(CoinLockInfoInner {
                sponsor,
                objects,
                unlock_time,
            }),
        }
    }
}

impl PartialOrd<Self> for CoinLockInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CoinLockInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.unlock_time.cmp(&other.inner.unlock_time)
    }
}
