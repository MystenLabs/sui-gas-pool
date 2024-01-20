// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::metrics::StoragePoolMetrics;
use crate::storage::{Storage, MAX_GAS_PER_QUERY};
use crate::types::{ExpirationTimeMs, GasCoin, GasGroupKey, ReservedGasGroup, UpdatedGasGroup};
use anyhow::bail;
use chrono::Utc;
use parking_lot::lock_api::MutexGuard;
use parking_lot::{Mutex, RawMutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use sui_types::base_types::{ObjectID, SuiAddress, VersionDigest};
use tracing::{debug, warn};
use typed_store::metrics::SamplingInterval;
use typed_store::rocks::{DBMap, MetricConf};
use typed_store::traits::{TableSummary, TypedStoreDebug};
use typed_store::Map;
use typed_store_derive::DBMapUtils;

pub struct RocksDBStorage {
    tables: Arc<RocksDBStorageTables>,
    mutexes: RwLock<HashMap<SuiAddress, Mutex<()>>>,
    metrics: Arc<StoragePoolMetrics>,
}

#[derive(DBMapUtils)]
struct RocksDBStorageTables {
    valid_addresses: DBMap<SuiAddress, ()>,
    available_gas_coins: DBMap<(SuiAddress, ObjectID), GasCoinInfo>,
    expiration_queue: DBMap<(SuiAddress, ExpirationTimeMs, GasGroupKey), ()>,
    reserved_gas_groups: DBMap<GasGroupKey, ReservedGasGroup>,
    pending_update_gas_groups: DBMap<GasGroupKey, ReservedGasGroup>,
}

impl RocksDBStorageTables {
    pub fn path(parent_path: &Path) -> PathBuf {
        parent_path.join("gas_pool")
    }

    pub fn open(parent_path: &Path) -> Self {
        Self::open_tables_read_write(
            Self::path(parent_path),
            MetricConf::default().with_sampling(SamplingInterval::new(Duration::from_secs(60), 0)),
            None,
            None,
        )
    }

    /// Take available gas coins until we have enough gas coins to satisfy the target budget,
    /// or we have gone over the per-query limit.
    /// If successful, we put them in the reserved gas coins table and the expiration queue.
    pub fn reserve_gas_coins(
        &self,
        _mutex_guard: MutexGuard<RawMutex, ()>,
        sponsor_address: SuiAddress,
        target_budget: u64,
        reserved_duration_ms: u64,
    ) -> anyhow::Result<Vec<GasCoin>> {
        if target_budget == 0 {
            bail!("Target budget must be non-zero");
        }
        let mut coins = vec![];
        let mut total_balance = 0;
        for ((addr, id), coin) in self.available_gas_coins.iter_with_bounds(
            Some((sponsor_address, ObjectID::ZERO)),
            Some((sponsor_address, ObjectID::MAX)),
        ) {
            assert_eq!(addr, sponsor_address);
            total_balance += coin.balance;
            coins.push((id, coin));
            if total_balance >= target_budget || coins.len() >= MAX_GAS_PER_QUERY {
                break;
            }
        }

        if total_balance < target_budget {
            warn!(
                "After taking {} gas coins, total balance {} is still less than target budget {}",
                coins.len(),
                total_balance,
                target_budget
            );
            bail!("Unable to find enough gas coins to meet the budget");
        }
        assert!(!coins.is_empty() && coins.len() <= MAX_GAS_PER_QUERY);

        let expiration_time = Utc::now().timestamp_millis() as u64 + reserved_duration_ms;
        let locked_gas_group = ReservedGasGroup {
            objects: coins.iter().map(|(id, _)| *id).collect(),
            expiration_time,
        };
        let group_key = locked_gas_group.get_key();
        let to_delete = locked_gas_group
            .objects
            .iter()
            .map(|o| (sponsor_address, *o));

        let mut batch = self.available_gas_coins.batch();
        batch.delete_batch(&self.available_gas_coins, to_delete)?;
        batch.insert_batch(&self.reserved_gas_groups, [(group_key, locked_gas_group)])?;
        batch.insert_batch(
            &self.expiration_queue,
            [((sponsor_address, expiration_time, group_key), ())],
        )?;
        batch.write()?;

        let coins = coins
            .into_iter()
            .map(|(object_id, coin)| coin.to_gas_coin(object_id))
            .collect();
        debug!(
            "Reserved coins in the storage for {:?}ms: {:?}",
            reserved_duration_ms, coins
        );
        Ok(coins)
    }

    pub fn ready_for_execution(
        &self,
        _mutex_guard: MutexGuard<RawMutex, ()>,
        sponsor_address: SuiAddress,
        gas_coins: BTreeSet<ObjectID>,
    ) -> anyhow::Result<()> {
        if gas_coins.is_empty() {
            bail!("Empty gas coins");
        }
        let group_key = *gas_coins.iter().next().unwrap();

        let Some(gas_group) = self.reserved_gas_groups.get(&group_key)? else {
            bail!("Gas object {:?} is not currently reserved", group_key);
        };
        if gas_group.objects != gas_coins {
            bail!(
                "Gas objects in the reservation ({:?}) don't match with the request: {:?}",
                gas_group.objects,
                gas_coins
            );
        }

        let mut batch = self.reserved_gas_groups.batch();
        batch.delete_batch(&self.reserved_gas_groups, [group_key])?;
        batch.delete_batch(
            &self.expiration_queue,
            [(sponsor_address, gas_group.expiration_time, group_key)],
        )?;
        batch.insert_batch(&self.pending_update_gas_groups, [(group_key, gas_group)])?;
        batch.write()?;

        debug!("Moved gas group to pending {:?}", group_key);

        Ok(())
    }

    pub fn expire_reserved_gas_groups(
        &self,
        mutex: &Mutex<()>,
        sponsor_address: SuiAddress,
    ) -> anyhow::Result<Vec<ReservedGasGroup>> {
        let cur_time = Utc::now().timestamp_millis() as u64;
        let expired: Vec<_> = self
            .expiration_queue
            .iter_with_bounds(
                Some((sponsor_address, 0, GasGroupKey::ZERO)),
                Some((sponsor_address, cur_time, GasGroupKey::MAX)),
            )
            .map(|(key, _)| key)
            .collect();
        let group_keys = expired.iter().map(|(_, _, key)| key);

        // We don't lock the mutex until now so that the iteration of the expiration_queue
        // doesn't occupy the mutex. This is an optimization. It is safe to do so because
        // even if two threads are calling this function at the same time, the processing of
        // each group will only happen once due to the mutex below.
        let _guard = mutex.lock();
        let still_reserved: Vec<_> = self
            .reserved_gas_groups
            .multi_get(group_keys)?
            .into_iter()
            .zip(&expired)
            .filter_map(|(group_opt, (_, exp_time, _))| {
                group_opt.and_then(|group| {
                    if &group.expiration_time == exp_time {
                        Some(group)
                    } else {
                        None
                    }
                })
            })
            .collect();

        let mut batch = self.reserved_gas_groups.batch();
        batch.delete_batch(
            &self.reserved_gas_groups,
            still_reserved.iter().map(|group| group.get_key()),
        )?;
        batch.delete_batch(&self.expiration_queue, &expired)?;
        batch.insert_batch(
            &self.pending_update_gas_groups,
            still_reserved
                .iter()
                .map(|group| (group.get_key(), group.clone())),
        )?;
        batch.write()?;

        debug!(
            "Moved coin groups to the pending table due to expiration: {:?}",
            still_reserved
        );

        Ok(still_reserved)
    }

    /// Add a list of available gas coins, and put it back to the available gas coins table.
    /// Also remove the entry from the reserved gas coins table.
    /// This function can be used both for releasing reserved gas coins and adding new gas coins.
    pub fn update_gas_coins(
        &self,
        _mutex_guard: MutexGuard<RawMutex, ()>,
        sponsor_address: SuiAddress,
        updated_gas_coins: Vec<UpdatedGasGroup>,
        metrics: &Arc<StoragePoolMetrics>,
    ) -> anyhow::Result<()> {
        let group_keys: Vec<_> = updated_gas_coins
            .iter()
            .map(|group| group.get_group_key())
            .collect();
        let new_coins = updated_gas_coins.iter().flat_map(|group| {
            group.updated_gas_coins.iter().map(|coin| {
                (
                    (sponsor_address, coin.object_ref.0),
                    GasCoinInfo::from_gas_coin(coin),
                )
            })
        });
        let is_all_pending = self
            .pending_update_gas_groups
            .multi_contains_keys(&group_keys)?;
        for (is_pending, group_key) in is_all_pending.into_iter().zip(&group_keys) {
            if !is_pending {
                let msg = format!(
                    "GroupKey not found in the pending_update_gas_groups table: {:?}",
                    group_key
                );
                metrics.invariant_violation(&msg);
                bail!(msg);
            }
        }
        let mut batch = self.available_gas_coins.batch();
        batch.insert_batch(&self.available_gas_coins, new_coins)?;
        batch.delete_batch(&self.pending_update_gas_groups, group_keys)?;
        batch.write()?;
        Ok(())
    }

    pub fn add_new_coins(
        &self,
        _mutex_guard: MutexGuard<RawMutex, ()>,
        sponsor_address: SuiAddress,
        new_coins: Vec<GasCoin>,
    ) -> anyhow::Result<()> {
        let mut batch = self.available_gas_coins.batch();
        batch.insert_batch(
            &self.available_gas_coins,
            new_coins.iter().map(|coin| {
                (
                    (sponsor_address, coin.object_ref.0),
                    GasCoinInfo::from_gas_coin(coin),
                )
            }),
        )?;
        batch.insert_batch(&self.valid_addresses, [(sponsor_address, ())])?;
        batch.write()?;
        Ok(())
    }

    fn iter_all_valid_addresses(&self) -> Vec<SuiAddress> {
        self.valid_addresses
            .unbounded_iter()
            .map(|(addr, _)| addr)
            .collect()
    }

    #[cfg(test)]
    fn iter_available_gas_coins(
        &self,
        sponsor_address: SuiAddress,
    ) -> impl Iterator<Item = GasCoin> + '_ {
        self.available_gas_coins
            .iter_with_bounds(
                Some((sponsor_address, ObjectID::ZERO)),
                Some((sponsor_address, ObjectID::MAX)),
            )
            .map(|((_, id), coin)| GasCoin {
                object_ref: (id, coin.version_digest.0, coin.version_digest.1),
                balance: coin.balance,
            })
    }

    #[cfg(test)]
    fn iter_reserved_gas_groups(&self) -> impl Iterator<Item = ReservedGasGroup> + '_ {
        self.reserved_gas_groups
            .unbounded_iter()
            .map(|(_, group)| group)
    }

    #[cfg(test)]
    fn iter_pending_update_gas_groups(&self) -> impl Iterator<Item = ReservedGasGroup> + '_ {
        self.pending_update_gas_groups
            .unbounded_iter()
            .map(|(_, group)| group)
    }
}

impl RocksDBStorage {
    pub fn new(parent_path: &Path, metrics: Arc<StoragePoolMetrics>) -> Self {
        let tables = Arc::new(RocksDBStorageTables::open(parent_path));
        let addresses = tables.iter_all_valid_addresses();
        let mutexes: HashMap<_, _> = addresses
            .into_iter()
            .map(|address| (address, Mutex::new(())))
            .collect();
        // TODO: Initiate all metrics with the correct values.
        Self {
            tables,
            mutexes: RwLock::new(mutexes),
            metrics,
        }
    }
}

#[async_trait::async_trait]
impl Storage for RocksDBStorage {
    async fn reserve_gas_coins(
        &self,
        sponsor_address: SuiAddress,
        target_budget: u64,
        reserved_duration_ms: u64,
    ) -> anyhow::Result<Vec<GasCoin>> {
        let read_guard = self.mutexes.read();
        let Some(mutex) = read_guard.get(&sponsor_address) else {
            bail!("Invalid sponsor address: {:?}", sponsor_address)
        };
        let gas_coins = self.tables.reserve_gas_coins(
            mutex.lock(),
            sponsor_address,
            target_budget,
            reserved_duration_ms,
        )?;
        self.metrics
            .cur_num_available_gas_coins
            .with_label_values(&[&sponsor_address.to_string()])
            .sub(gas_coins.len() as i64);
        self.metrics
            .cur_num_reserved_gas_coins
            .with_label_values(&[&sponsor_address.to_string()])
            .add(gas_coins.len() as i64);
        self.metrics
            .cur_total_available_gas_balance
            .with_label_values(&[&sponsor_address.to_string()])
            .sub(gas_coins.iter().map(|c| c.balance as i64).sum());
        Ok(gas_coins)
    }

    async fn read_for_execution(
        &self,
        sponsor_address: SuiAddress,
        gas_coins: BTreeSet<ObjectID>,
    ) -> anyhow::Result<()> {
        let read_guard = self.mutexes.read();
        let Some(mutex) = read_guard.get(&sponsor_address) else {
            bail!("Invalid sponsor address: {:?}", sponsor_address)
        };
        self.tables
            .ready_for_execution(mutex.lock(), sponsor_address, gas_coins)
    }

    async fn update_gas_coins(
        &self,
        sponsor_address: SuiAddress,
        updated_gas_coins: Vec<UpdatedGasGroup>,
    ) -> anyhow::Result<()> {
        let read_guard = self.mutexes.read();
        let Some(mutex) = read_guard.get(&sponsor_address) else {
            bail!("Invalid sponsor address: {:?}", sponsor_address)
        };
        let (released_gas_coins_len, released_gas_coin_balance, deleted_gas_coins_len) =
            updated_gas_coins
                .iter()
                .fold((0, 0, 0), |cur_value, group| {
                    (
                        cur_value.0 + group.updated_gas_coins.len(),
                        cur_value.1
                            + group
                                .updated_gas_coins
                                .iter()
                                .map(|c| c.balance)
                                .sum::<u64>(),
                        cur_value.2 + group.deleted_gas_coins.len(),
                    )
                });

        self.tables.update_gas_coins(
            mutex.lock(),
            sponsor_address,
            updated_gas_coins,
            &self.metrics,
        )?;
        self.metrics
            .cur_num_available_gas_coins
            .with_label_values(&[&sponsor_address.to_string()])
            .add(released_gas_coins_len as i64);
        self.metrics
            .cur_num_reserved_gas_coins
            .with_label_values(&[&sponsor_address.to_string()])
            .sub((released_gas_coins_len + deleted_gas_coins_len) as i64);
        self.metrics
            .cur_total_available_gas_balance
            .with_label_values(&[&sponsor_address.to_string()])
            .add(released_gas_coin_balance as i64);
        Ok(())
    }

    async fn add_new_coins(
        &self,
        sponsor_address: SuiAddress,
        new_coins: Vec<GasCoin>,
    ) -> anyhow::Result<()> {
        if !self.mutexes.read().contains_key(&sponsor_address) {
            self.mutexes.write().insert(sponsor_address, Mutex::new(()));
        }
        let (new_coins_len, new_coins_balance) =
            new_coins.iter().fold((0, 0), |cur_value, coin| {
                (cur_value.0 + 1, cur_value.1 + coin.balance)
            });
        self.tables.add_new_coins(
            // unwrap safe because we would add it above if not exist.
            self.mutexes.read().get(&sponsor_address).unwrap().lock(),
            sponsor_address,
            new_coins,
        )?;
        self.metrics
            .cur_num_available_gas_coins
            .with_label_values(&[&sponsor_address.to_string()])
            .add(new_coins_len);
        self.metrics
            .cur_total_available_gas_balance
            .with_label_values(&[&sponsor_address.to_string()])
            .add(new_coins_balance as i64);
        Ok(())
    }

    async fn expire_coins(
        &self,
        sponsor_address: SuiAddress,
    ) -> anyhow::Result<Vec<BTreeSet<ObjectID>>> {
        let read_guard = self.mutexes.read();
        let Some(mutex) = read_guard.get(&sponsor_address) else {
            bail!("Invalid sponsor address: {:?}", sponsor_address)
        };
        let expired = self
            .tables
            .expire_reserved_gas_groups(mutex, sponsor_address)?;
        Ok(expired.into_iter().map(|group| group.objects).collect())
    }

    async fn initialized(&self, sponsor_address: SuiAddress) -> anyhow::Result<bool> {
        Ok(self.tables.valid_addresses.get(&sponsor_address)?.is_some())
    }

    async fn check_health(&self) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(test)]
    async fn get_available_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        self.tables
            .iter_available_gas_coins(sponsor_address)
            .count()
    }

    #[cfg(test)]
    async fn get_total_available_coin_balance(&self, sponsor_address: SuiAddress) -> u64 {
        self.tables
            .iter_available_gas_coins(sponsor_address)
            .map(|c| c.balance)
            .sum()
    }

    #[cfg(test)]
    async fn get_reserved_coin_count(&self) -> usize {
        self.tables
            .iter_reserved_gas_groups()
            .map(|group| group.objects.len())
            .sum()
    }

    #[cfg(test)]
    async fn get_pending_update_coin_count(&self) -> usize {
        self.tables.iter_pending_update_gas_groups().count()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GasCoinInfo {
    version_digest: VersionDigest,
    balance: u64,
}

impl GasCoinInfo {
    pub fn from_gas_coin(gas_coin: &GasCoin) -> Self {
        Self {
            version_digest: (gas_coin.object_ref.1, gas_coin.object_ref.2),
            balance: gas_coin.balance,
        }
    }

    pub fn to_gas_coin(&self, object_id: ObjectID) -> GasCoin {
        GasCoin {
            object_ref: (object_id, self.version_digest.0, self.version_digest.1),
            balance: self.balance,
        }
    }
}
