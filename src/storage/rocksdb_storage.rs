// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::storage::Storage;
use crate::types::GasCoin;
use chrono::Utc;
use parking_lot::lock_api::MutexGuard;
use parking_lot::{Mutex, RawMutex};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use sui_types::base_types::ObjectID;
use tracing::warn;
use typed_store::metrics::SamplingInterval;
use typed_store::rocks::{DBMap, MetricConf};
use typed_store::traits::{TableSummary, TypedStoreDebug};
use typed_store::Map;
use typed_store_derive::DBMapUtils;

const MAX_GAS_PER_QUERY: usize = 256;

// TODO: Add more logging and metrics

pub struct RocksDBStorage {
    tables: Arc<RocksDBStorageTables>,
    /// This mutex is used to ensure operations on the available coin queue are atomic.
    mutex: Arc<Mutex<()>>,
}

type ReserveTimeMs = u64;

#[derive(DBMapUtils)]
struct RocksDBStorageTables {
    available_gas_coins: DBMap<u64, GasCoin>,
    /// Clients are expected to call `put_back_gas_coin` within a certain amount of time after calling `take_first_available_gas_coin`.
    /// We still keep the reserved_gas_coins list such that even if in the event that
    /// sometimes clients crash and don't release them, we can run a GC process to release them
    /// from time to time.
    reserved_gas_coins: DBMap<ObjectID, ReserveTimeMs>,
}

impl RocksDBStorageTables {
    pub fn path(parent_path: &Path) -> PathBuf {
        parent_path.join("gas_pool")
    }

    pub fn open(parent_path: &Path) -> Self {
        Self::open_tables_read_write(
            Self::path(parent_path),
            MetricConf::with_sampling(SamplingInterval::new(Duration::from_secs(60), 0)),
            None,
            None,
        )
    }

    /// Take the first available gas coins with the smallest index,
    /// until we have enough gas coins to satisfy the target budget, or we have gone
    /// over the per-query limit.
    /// If successful, we put them in the reserved gas coins table.
    pub fn take_first_available_gas_coins(
        &self,
        mutex: &Mutex<()>,
        target_budget: u64,
    ) -> Vec<GasCoin> {
        let coins = {
            // This iteration and removal must be atomic to avoid data races.
            let _guard = mutex.lock();
            let mut indexes = vec![];
            let mut coins = vec![];
            let mut total_balance = 0;
            for (index, coin) in self.available_gas_coins.unbounded_iter() {
                total_balance += coin.balance;
                coins.push(coin);
                indexes.push(index);
                if coins.len() >= MAX_GAS_PER_QUERY || total_balance >= target_budget {
                    break;
                }
            }
            if total_balance < target_budget {
                warn!(
                    "After taking {} gas coins, total balance {} is still less than target budget {}",
                    coins.len(), total_balance, target_budget
                );
                vec![]
            } else {
                let mut batch = self.available_gas_coins.batch();
                batch
                    .delete_batch(&self.available_gas_coins, indexes)
                    .unwrap();
                batch.write().unwrap();
                coins
            }
        };
        let mut batch = self.reserved_gas_coins.batch();
        let cur_time = Utc::now().timestamp_millis() as u64;
        batch
            .insert_batch(
                &self.reserved_gas_coins,
                coins.iter().map(|c| (c.object_ref.0, cur_time)),
            )
            .unwrap();
        batch.write().unwrap();
        coins
    }

    /// Add a list of available gas coins, and put it at the end of the available gas coins queue.
    /// This is done by getting the next index in the available gas coins table, and inserting the coin at that index.
    /// This function can be used both for releasing reserved gas coins and adding new gas coins.
    pub fn add_gas_coins(&self, mutex: &Mutex<()>, gas_coins: Vec<GasCoin>) {
        {
            let guard = mutex.lock();
            let next_index = self.get_next_available_gas_coin_index(&guard);
            let mut batch = self.available_gas_coins.batch();
            batch
                .insert_batch(
                    &self.available_gas_coins,
                    gas_coins
                        .iter()
                        .enumerate()
                        .map(|(offset, c)| (next_index + offset as u64, c)),
                )
                .unwrap();
            batch.write().unwrap();
        }
        let mut batch = self.reserved_gas_coins.batch();
        batch
            .delete_batch(
                &self.reserved_gas_coins,
                gas_coins.iter().map(|c| c.object_ref.0),
            )
            .unwrap();
        batch.write().unwrap();
    }

    fn get_next_available_gas_coin_index(&self, _mutex_guard: &MutexGuard<RawMutex, ()>) -> u64 {
        self.available_gas_coins
            .unbounded_iter()
            .skip_to_last()
            .next()
            .map(|(idx, _)| idx + 1)
            .unwrap_or(0)
    }
}

impl RocksDBStorage {
    pub fn new(parent_path: &Path) -> Self {
        Self {
            tables: Arc::new(RocksDBStorageTables::open(parent_path)),
            mutex: Arc::new(Mutex::new(())),
        }
    }
}

impl Storage for RocksDBStorage {
    fn reserve_gas_coins(&self, target_budget: u64) -> Vec<GasCoin> {
        // TODO: Check target_budget is valid value.
        self.tables
            .take_first_available_gas_coins(self.mutex.as_ref(), target_budget)
    }

    fn add_gas_coins(&self, gas_coin: Vec<GasCoin>) {
        self.tables.add_gas_coins(self.mutex.as_ref(), gas_coin);
    }

    #[cfg(test)]
    fn get_available_coin_count(&self) -> usize {
        self.tables.available_gas_coins.unbounded_iter().count()
    }

    #[cfg(test)]
    fn get_reserved_coin_count(&self) -> usize {
        self.tables.reserved_gas_coins.unbounded_iter().count()
    }

    #[cfg(test)]
    fn get_total_available_coin_balance(&self) -> u64 {
        self.tables
            .available_gas_coins
            .unbounded_iter()
            .map(|(_, c)| c.balance)
            .sum()
    }
}
