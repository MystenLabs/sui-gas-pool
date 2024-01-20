// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasPoolStorageConfig;
use crate::metrics::StorageMetrics;
use crate::storage::redis::RedisStorage;
use crate::types::{GasCoin, ReservationID};
use std::sync::Arc;
use sui_types::base_types::{ObjectID, SuiAddress};

mod redis;

pub const MAX_GAS_PER_QUERY: usize = 256;

/// Defines the trait for a storage that manages gas coins.
/// It is expected to support concurrent access and manage atomicity internally.
/// It supports multiple addresses each with its own gas coin queue.
#[async_trait::async_trait]
pub trait Storage: Sync + Send {
    /// Reserve gas coins for the given sponsor address, with total coin balance >= target_budget.
    /// If there is not enough balance, returns error.
    /// The implementation is required to guarantee that:
    /// 1. It never returns the same coin to multiple callers.
    /// 2. It keeps a record of the reserved coins with timestamp, so that in the case
    ///    when caller forgets to release them, some cleanup process can clean them up latter.
    /// 3. It should never return more than 256 coins at a time since that's the upper bound of gas.
    async fn reserve_gas_coins(
        &self,
        sponsor_address: SuiAddress,
        target_budget: u64,
        reserved_duration_ms: u64,
    ) -> anyhow::Result<(ReservationID, Vec<GasCoin>)>;

    async fn ready_for_execution(
        &self,
        sponsor_address: SuiAddress,
        reservation_id: ReservationID,
    ) -> anyhow::Result<()>;

    async fn add_new_coins(
        &self,
        sponsor_address: SuiAddress,
        new_coins: Vec<GasCoin>,
    ) -> anyhow::Result<()>;

    async fn expire_coins(&self, sponsor_address: SuiAddress) -> anyhow::Result<Vec<ObjectID>>;

    async fn check_health(&self) -> anyhow::Result<()>;

    #[cfg(test)]
    async fn flush_db(&self);

    async fn get_available_coin_count(&self, sponsor_address: SuiAddress) -> usize;

    #[cfg(test)]
    async fn get_available_coin_total_balance(&self, sponsor_address: SuiAddress) -> u64;

    #[cfg(test)]
    async fn get_reserved_coin_count(&self, sponsor_address: SuiAddress) -> usize;

    // TODO: Add APIs to support collecting coins that were forgotten to be released.
}

pub async fn connect_storage(
    config: &GasPoolStorageConfig,
    metrics: Arc<StorageMetrics>,
) -> Arc<dyn Storage> {
    let storage: Arc<dyn Storage> = match config {
        GasPoolStorageConfig::Redis { redis_url } => {
            Arc::new(RedisStorage::new(redis_url, metrics))
        }
    };
    storage
        .check_health()
        .await
        .expect("Unable to connect to the storage layer");
    storage
}

#[cfg(test)]
pub async fn connect_storage_for_testing_with_config(
    config: &GasPoolStorageConfig,
) -> Arc<dyn Storage> {
    let storage = connect_storage(config, StorageMetrics::new_for_testing()).await;
    storage.flush_db().await;
    storage
}

#[cfg(test)]
pub async fn connect_storage_for_testing() -> Arc<dyn Storage> {
    connect_storage_for_testing_with_config(&GasPoolStorageConfig::default()).await
}

#[cfg(test)]
mod tests {
    use crate::storage::{connect_storage_for_testing, Storage, MAX_GAS_PER_QUERY};
    use crate::types::GasCoin;
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;
    use sui_types::base_types::{random_object_ref, SuiAddress};

    async fn assert_coin_count(
        storage: &Arc<dyn Storage>,
        sponsor_address: SuiAddress,
        available: usize,
        reserved: usize,
    ) {
        assert_eq!(
            storage.get_available_coin_count(sponsor_address).await,
            available
        );
        assert_eq!(
            storage.get_reserved_coin_count(sponsor_address).await,
            reserved
        );
    }

    async fn setup(init_balance: Vec<(SuiAddress, Vec<u64>)>) -> Arc<dyn Storage> {
        let storage = connect_storage_for_testing().await;
        for (sponsor, amounts) in init_balance {
            let gas_coins = amounts
                .into_iter()
                .map(|balance| GasCoin {
                    object_ref: random_object_ref(),
                    balance,
                })
                .collect::<Vec<_>>();
            for chunk in gas_coins.chunks(5000) {
                storage
                    .add_new_coins(sponsor, chunk.to_vec())
                    .await
                    .unwrap();
            }
        }
        storage
    }

    #[tokio::test]
    async fn test_successful_reservation() {
        // Create a gas pool of 100000 coins, each with balance of 1.
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100000])]).await;
        assert_coin_count(&storage, sponsor, 100000, 0).await;
        let mut cur_available = 100000;
        let mut expected_res_id = 1;
        for i in 1..=MAX_GAS_PER_QUERY {
            let (res_id, reserved_gas_coins) = storage
                .reserve_gas_coins(sponsor, i as u64, 1000)
                .await
                .unwrap();
            assert_eq!(expected_res_id, res_id);
            assert_eq!(reserved_gas_coins.len(), i);
            expected_res_id += 1;
            cur_available -= i;
        }
        assert_coin_count(&storage, sponsor, cur_available, 100000 - cur_available).await;
    }

    #[tokio::test]
    async fn test_max_gas_coin_per_query() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; MAX_GAS_PER_QUERY + 1])]).await;
        assert!(storage
            .reserve_gas_coins(sponsor, (MAX_GAS_PER_QUERY + 1) as u64, 1000)
            .await
            .is_err());
        assert_coin_count(&storage, sponsor, MAX_GAS_PER_QUERY + 1, 0).await;
    }

    #[tokio::test]
    async fn test_insufficient_pool_budget() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        assert!(storage.reserve_gas_coins(sponsor, 101, 1000).await.is_err());
        assert_coin_count(&storage, sponsor, 100, 0).await;
    }

    #[tokio::test]
    async fn test_coin_release() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        for _ in 0..100 {
            // Keep reserving and putting them back.
            // Should be able to repeat this process indefinitely if balance are not changed.
            let (res_id, reserved_gas_coins) =
                storage.reserve_gas_coins(sponsor, 99, 1000).await.unwrap();
            assert_eq!(reserved_gas_coins.len(), 99);
            assert_coin_count(&storage, sponsor, 1, 99).await;
            storage.ready_for_execution(sponsor, res_id).await.unwrap();
            storage
                .add_new_coins(sponsor, reserved_gas_coins)
                .await
                .unwrap();
            assert_coin_count(&storage, sponsor, 100, 0).await;
        }
    }

    #[tokio::test]
    async fn test_coin_release_with_updated_balance() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        for _ in 0..10 {
            let (res_id, mut reserved_gas_coins) =
                storage.reserve_gas_coins(sponsor, 10, 1000).await.unwrap();
            assert_eq!(
                reserved_gas_coins.iter().map(|c| c.balance).sum::<u64>(),
                10
            );
            for reserved_gas_coin in reserved_gas_coins.iter_mut() {
                if reserved_gas_coin.balance > 0 {
                    reserved_gas_coin.balance -= 1;
                }
            }
            storage.ready_for_execution(sponsor, res_id).await.unwrap();
            storage
                .add_new_coins(sponsor, reserved_gas_coins)
                .await
                .unwrap();
        }
        assert_coin_count(&storage, sponsor, 100, 0).await;
        assert_eq!(storage.get_available_coin_total_balance(sponsor).await, 0);
        assert!(storage.reserve_gas_coins(sponsor, 1, 1000).await.is_err());
    }

    #[tokio::test]
    async fn test_deleted_objects() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        let (res_id, mut reserved_gas_coins) =
            storage.reserve_gas_coins(sponsor, 100, 1000).await.unwrap();
        assert_eq!(reserved_gas_coins.len(), 100);

        storage.ready_for_execution(sponsor, res_id).await.unwrap();

        reserved_gas_coins.drain(0..50);
        storage
            .add_new_coins(sponsor, reserved_gas_coins)
            .await
            .unwrap();
        assert_coin_count(&storage, sponsor, 50, 0).await;
    }

    #[tokio::test]
    async fn test_coin_expiration() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        let (_res_id1, reserved_gas_coins1) =
            storage.reserve_gas_coins(sponsor, 10, 900).await.unwrap();
        assert_eq!(reserved_gas_coins1.len(), 10);
        let (_res_id2, reserved_gas_coins2) =
            storage.reserve_gas_coins(sponsor, 30, 1900).await.unwrap();
        assert_eq!(reserved_gas_coins2.len(), 30);
        // Just to make sure these two reservations will have a different expiration timestamp.
        tokio::time::sleep(Duration::from_millis(1)).await;
        let (_res_id3, reserved_gas_coins3) =
            storage.reserve_gas_coins(sponsor, 50, 1900).await.unwrap();
        assert_eq!(reserved_gas_coins3.len(), 50);
        assert_coin_count(&storage, sponsor, 10, 90).await;

        assert!(storage.expire_coins(sponsor).await.unwrap().is_empty());
        assert_coin_count(&storage, sponsor, 10, 90).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let expired1 = storage.expire_coins(sponsor).await.unwrap();
        assert_eq!(expired1.len(), 10);
        assert_eq!(
            expired1.iter().cloned().collect::<BTreeSet<_>>(),
            reserved_gas_coins1
                .iter()
                .map(|coin| coin.object_ref.0)
                .collect::<BTreeSet<_>>()
        );
        assert_coin_count(&storage, sponsor, 10, 80).await;

        assert!(storage.expire_coins(sponsor).await.unwrap().is_empty());
        assert_coin_count(&storage, sponsor, 10, 80).await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        let expired2 = storage.expire_coins(sponsor).await.unwrap();
        assert_eq!(expired2.len(), 80);
        assert_eq!(
            expired2.iter().cloned().collect::<BTreeSet<_>>(),
            reserved_gas_coins2
                .iter()
                .chain(&reserved_gas_coins3)
                .map(|coin| coin.object_ref.0)
                .collect::<BTreeSet<_>>()
        );
        assert_coin_count(&storage, sponsor, 10, 0).await;
    }

    #[tokio::test]
    async fn test_multiple_sponsors() {
        let sponsors = (0..10)
            .map(|_| SuiAddress::random_for_testing_only())
            .collect::<Vec<_>>();
        let storage = setup(
            sponsors
                .iter()
                .map(|sponsor| (*sponsor, vec![1; 100]))
                .collect(),
        )
        .await;
        for sponsor in sponsors.iter() {
            let (_, gas_coins) = storage.reserve_gas_coins(*sponsor, 50, 1000).await.unwrap();
            assert_eq!(gas_coins.len(), 50);
            assert_coin_count(&storage, *sponsor, 50, 50).await;
        }
    }

    #[tokio::test]
    async fn test_invalid_sponsor() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100])]).await;
        assert!(storage
            .reserve_gas_coins(SuiAddress::random_for_testing_only(), 1, 1000)
            .await
            .is_err());
        assert_eq!(
            storage
                .reserve_gas_coins(sponsor, 1, 1000)
                .await
                .unwrap()
                .1
                .len(),
            1
        )
    }

    #[tokio::test]
    async fn test_concurrent_reservation() {
        let sponsor = SuiAddress::random_for_testing_only();
        let storage = setup(vec![(sponsor, vec![1; 100000])]).await;
        let mut handles = vec![];
        for _ in 0..10 {
            let storage = storage.clone();
            let sponsor = sponsor;
            handles.push(tokio::spawn(async move {
                let mut reserved_gas_coins = vec![];
                for _ in 0..100 {
                    let (_, newly_reserved) =
                        storage.reserve_gas_coins(sponsor, 3, 1000).await.unwrap();
                    reserved_gas_coins.extend(newly_reserved);
                }
                reserved_gas_coins
            }));
        }
        let mut reserved_gas_coins = vec![];
        for handle in handles {
            reserved_gas_coins.extend(handle.await.unwrap());
        }
        let count = reserved_gas_coins.len();
        // Check that all object IDs are unique in all reservations.
        reserved_gas_coins.sort_by_key(|c| c.object_ref.0);
        reserved_gas_coins.dedup_by_key(|c| c.object_ref.0);
        assert_eq!(reserved_gas_coins.len(), count);
        assert_coin_count(&storage, sponsor, 100000 - count, count).await;
    }
}
