// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::GasPoolStorageConfig;
use crate::types::GasCoin;
use std::sync::Arc;

mod rocksdb_storage;

// TODO: Support multiple sponsor addresses.

pub trait Storage: Sync + Send {
    fn reserve_gas_coins(&self, target_budget: u64) -> Vec<GasCoin>;

    fn add_gas_coins(&self, gas_coins: Vec<GasCoin>);

    #[cfg(test)]
    fn get_available_coin_count(&self) -> usize;

    #[cfg(test)]
    fn get_reserved_coin_count(&self) -> usize;

    #[cfg(test)]
    fn get_total_available_coin_balance(&self) -> u64;

    // TODO: Add APIs to GC coins.
}

pub fn connect_storage(config: &GasPoolStorageConfig) -> Arc<dyn Storage> {
    let GasPoolStorageConfig::RocksDb { db_path } = config;
    Arc::new(rocksdb_storage::RocksDBStorage::new(db_path))
}

#[cfg(test)]
mod tests {
    use crate::config::GasPoolStorageConfig;
    use crate::storage::{connect_storage, Storage};
    use crate::types::GasCoin;
    use rand::Rng;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use sui_types::base_types::random_object_ref;

    fn assert_coin_count(storage: &Arc<dyn Storage>, available: usize, reserved: usize) {
        assert_eq!(storage.get_available_coin_count(), available);
        assert_eq!(storage.get_reserved_coin_count(), reserved);
    }

    #[tokio::test]
    async fn test_empty_store() {
        let storage = connect_storage(&GasPoolStorageConfig::default());
        assert_coin_count(&storage, 0, 0);
        assert!(storage.reserve_gas_coins(1).is_empty());
    }

    #[tokio::test]
    async fn test_basic_flow() {
        let storage = connect_storage(&GasPoolStorageConfig::default());
        let gas_coin = GasCoin {
            object_ref: random_object_ref(),
            balance: 1000,
        };
        storage.add_gas_coins(vec![gas_coin.clone()]);
        assert_coin_count(&storage, 1, 0);

        let mut reserved_gas_coins = storage.reserve_gas_coins(1000);
        assert_eq!(reserved_gas_coins.len(), 1);
        let mut reserved_gas_coin = reserved_gas_coins.pop().unwrap();
        assert_eq!(gas_coin, reserved_gas_coin);
        assert_coin_count(&storage, 0, 1);
        assert!(storage.reserve_gas_coins(1).is_empty());

        reserved_gas_coin.balance = 1;
        storage.add_gas_coins(vec![reserved_gas_coin.clone()]);
        assert_coin_count(&storage, 1, 0);

        let another_reserved_gas_coin = storage.reserve_gas_coins(1).pop().unwrap();
        assert_eq!(another_reserved_gas_coin, reserved_gas_coin);
        assert_coin_count(&storage, 0, 1);
        assert!(storage.reserve_gas_coins(1).is_empty());
    }

    #[tokio::test]
    async fn test_budget() {
        let storage = connect_storage(&GasPoolStorageConfig::default());
        let gas_coins = (0..10)
            .map(|_| GasCoin {
                object_ref: random_object_ref(),
                balance: 1000,
            })
            .collect::<Vec<_>>();
        storage.add_gas_coins(gas_coins);
        assert_coin_count(&storage, 10, 0);

        // Requires 3 coins to hit 2999 budget.
        let reserved_gas_coins = storage.reserve_gas_coins(2999);
        assert_eq!(reserved_gas_coins.len(), 3);
        assert_coin_count(&storage, 7, 3);

        storage.add_gas_coins(reserved_gas_coins);
        assert_coin_count(&storage, 10, 0);

        // No way to meet budget of 10001 since total balance is only 10000.
        let reserved_gas_coins = storage.reserve_gas_coins(10001);
        assert!(reserved_gas_coins.is_empty());
        assert_coin_count(&storage, 10, 0);
    }

    #[tokio::test]
    async fn stress_test() {
        let storage = connect_storage(&GasPoolStorageConfig::default());
        const COIN_COUNT: u64 = 100;
        const INIT_COIN_BALANCE: u64 = 1000;
        const DELTA: u64 = 100;
        const EXPECTED_TOTAL_RESERVE_COUNT: u64 = COIN_COUNT * INIT_COIN_BALANCE / DELTA;
        let gas_coins = (0..COIN_COUNT)
            .map(|_| GasCoin {
                object_ref: random_object_ref(),
                balance: INIT_COIN_BALANCE,
            })
            .collect::<Vec<_>>();
        storage.add_gas_coins(gas_coins);
        let total_reserve_count = Arc::new(AtomicU64::default());
        let processes = (0..10).map(|_| {
            let storage = storage.clone();
            let total_reserve_count = total_reserve_count.clone();
            tokio::spawn(async move {
                let mut rng = rand::thread_rng();
                loop {
                    if total_reserve_count.load(std::sync::atomic::Ordering::Relaxed)
                        == EXPECTED_TOTAL_RESERVE_COUNT
                    {
                        break;
                    }
                    // Reserve with a somewhat random budget.
                    let mut reserved_gas_coins =
                        storage.reserve_gas_coins(rng.gen_range(1..=5) * DELTA);
                    for reserved_gas_coin in reserved_gas_coins.iter_mut() {
                        // Consume each coin balance by DELTA.
                        if reserved_gas_coin.balance >= DELTA {
                            reserved_gas_coin.balance -= DELTA;
                            total_reserve_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    storage.add_gas_coins(reserved_gas_coins);
                }
            })
        });
        for process in processes {
            process.await.unwrap();
        }
        assert_coin_count(&storage, COIN_COUNT as usize, 0);
        assert_eq!(storage.get_total_available_coin_balance(), 0);
    }
}
