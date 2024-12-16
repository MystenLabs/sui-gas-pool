// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

mod script_manager;

use crate::metrics::StorageMetrics;
use crate::storage::redis::script_manager::ScriptManager;
use crate::storage::Storage;
use crate::types::{GasCoin, ReservationID};
use chrono::Utc;
use redis::aio::ConnectionManager;
use std::ops::Add;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use sui_types::base_types::{ObjectDigest, ObjectID, SequenceNumber, SuiAddress};
use tracing::{debug, info};

pub struct RedisStorage {
    conn_manager: ConnectionManager,
    metrics: Arc<StorageMetrics>,
}

impl RedisStorage {
    pub async fn new(redis_url: &str, metrics: Arc<StorageMetrics>) -> Self {
        let client = redis::Client::open(redis_url).unwrap();
        let conn_manager = ConnectionManager::new(client).await.unwrap();
        Self {
            conn_manager,
            metrics,
        }
    }
}

#[async_trait::async_trait]
impl Storage for RedisStorage {
    async fn reserve_gas_coins(
        &self,
        sponsor_address: SuiAddress,
        target_budget: u64,
        reserved_duration_ms: u64,
    ) -> anyhow::Result<(ReservationID, Vec<GasCoin>)> {
        self.metrics.num_reserve_gas_coins_requests.inc();
        let sponsor_str = sponsor_address.to_string();

        let expiration_time = Utc::now()
            .add(Duration::from_millis(reserved_duration_ms))
            .timestamp_millis() as u64;
        let mut conn = self.conn_manager.clone();
        let (reservation_id, coins, new_total_balance, new_coin_count): (
            ReservationID,
            Vec<String>,
            i64,
            i64,
        ) = ScriptManager::reserve_gas_coins_script()
            .arg(sponsor_str.clone())
            .arg(target_budget)
            .arg(expiration_time)
            .invoke_async(&mut conn)
            .await?;
        // The script returns (0, []) if it is unable to find enough coins to reserve.
        // We choose to handle the error here instead of inside the script so that we could
        // provide a more readable error message.
        if coins.is_empty() {
            return Err(anyhow::anyhow!(
                "Unable to reserve gas coins for the given budget."
            ));
        }
        let gas_coins: Vec<_> = coins
            .into_iter()
            .map(|s| {
                // Each coin is in the form of: balance,object_id,version,digest
                let mut splits = s.split(',');
                let balance = splits.next().unwrap().parse::<u64>().unwrap();
                let object_id = ObjectID::from_str(splits.next().unwrap()).unwrap();
                let version = SequenceNumber::from(splits.next().unwrap().parse::<u64>().unwrap());
                let digest = ObjectDigest::from_str(splits.next().unwrap()).unwrap();
                GasCoin {
                    balance,
                    object_ref: (object_id, version, digest),
                }
            })
            .collect();

        self.metrics
            .gas_pool_available_gas_coin_count
            .with_label_values(&[&sponsor_str])
            .set(new_coin_count);
        self.metrics
            .gas_pool_available_gas_total_balance
            .with_label_values(&[&sponsor_str])
            .set(new_total_balance);
        self.metrics.num_successful_reserve_gas_coins_requests.inc();
        Ok((reservation_id, gas_coins))
    }

    async fn ready_for_execution(
        &self,
        sponsor_address: SuiAddress,
        reservation_id: ReservationID,
    ) -> anyhow::Result<()> {
        self.metrics.num_ready_for_execution_requests.inc();
        let sponsor_str = sponsor_address.to_string();

        let mut conn = self.conn_manager.clone();
        ScriptManager::ready_for_execution_script()
            .arg(sponsor_str)
            .arg(reservation_id)
            .invoke_async::<_, ()>(&mut conn)
            .await?;

        self.metrics
            .num_successful_ready_for_execution_requests
            .inc();
        Ok(())
    }

    async fn add_new_coins(
        &self,
        sponsor_address: SuiAddress,
        new_coins: Vec<GasCoin>,
    ) -> anyhow::Result<()> {
        self.metrics.num_add_new_coins_requests.inc();
        let sponsor_str = sponsor_address.to_string();
        let formatted_coins = new_coins
            .iter()
            .map(|c| {
                // The format is: balance,object_id,version,digest
                // The way we turn them into strings must be consistent with the way we parse them in
                // reserve_gas_coins_script.
                format!(
                    "{},{},{},{}",
                    c.balance,
                    c.object_ref.0,
                    c.object_ref.1.value(),
                    c.object_ref.2
                )
            })
            .collect::<Vec<String>>();

        let mut conn = self.conn_manager.clone();
        let (new_total_balance, new_coin_count): (i64, i64) = ScriptManager::add_new_coins_script()
            .arg(sponsor_str.clone())
            .arg(serde_json::to_string(&formatted_coins)?)
            .invoke_async(&mut conn)
            .await?;

        debug!(
            "After add_new_coins. New total balance: {}, new coin count: {}",
            new_total_balance, new_coin_count
        );
        self.metrics
            .gas_pool_available_gas_coin_count
            .with_label_values(&[&sponsor_str])
            .set(new_coin_count);
        self.metrics
            .gas_pool_available_gas_total_balance
            .with_label_values(&[&sponsor_str])
            .set(new_total_balance);
        self.metrics.num_successful_add_new_coins_requests.inc();
        Ok(())
    }

    async fn expire_coins(&self, sponsor_address: SuiAddress) -> anyhow::Result<Vec<ObjectID>> {
        self.metrics.num_expire_coins_requests.inc();
        let sponsor_str = sponsor_address.to_string();

        let now = Utc::now().timestamp_millis() as u64;
        let mut conn = self.conn_manager.clone();
        let expired_coin_strings: Vec<String> = ScriptManager::expire_coins_script()
            .arg(sponsor_str)
            .arg(now)
            .invoke_async(&mut conn)
            .await?;
        // The script returns a list of comma separated coin ids.
        let expired_coin_ids = expired_coin_strings
            .iter()
            .flat_map(|s| s.split(',').map(|id| ObjectID::from_str(id).unwrap()))
            .collect();

        self.metrics.num_successful_expire_coins_requests.inc();
        Ok(expired_coin_ids)
    }

    async fn init_coin_stats_at_startup(
        &self,
        sponsor_address: SuiAddress,
    ) -> anyhow::Result<(u64, u64)> {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        let (available_coin_count, available_coin_total_balance): (i64, i64) =
            ScriptManager::init_coin_stats_at_startup_script()
                .arg(sponsor_str.clone())
                .invoke_async(&mut conn)
                .await?;
        info!(
            sponsor_address=?sponsor_str,
            "Number of available gas coins in the pool: {}, total balance: {}",
            available_coin_count,
            available_coin_total_balance
        );
        self.metrics
            .gas_pool_available_gas_coin_count
            .with_label_values(&[&sponsor_str])
            .set(available_coin_count);
        self.metrics
            .gas_pool_available_gas_total_balance
            .with_label_values(&[&sponsor_str])
            .set(available_coin_total_balance);
        Ok((
            available_coin_count as u64,
            available_coin_total_balance as u64,
        ))
    }

    async fn is_initialized(&self, sponsor_address: SuiAddress) -> anyhow::Result<bool> {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        let result = ScriptManager::get_is_initialized_script()
            .arg(sponsor_str)
            .invoke_async::<_, bool>(&mut conn)
            .await?;
        Ok(result)
    }

    async fn acquire_init_lock(
        &self,
        sponsor_address: SuiAddress,
        lock_duration_sec: u64,
    ) -> anyhow::Result<bool> {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        let cur_timestamp = Utc::now().timestamp() as u64;
        debug!(
            "Acquiring init lock at {} for {} seconds",
            cur_timestamp, lock_duration_sec
        );
        let result = ScriptManager::acquire_init_lock_script()
            .arg(sponsor_str)
            .arg(cur_timestamp)
            .arg(lock_duration_sec)
            .invoke_async::<_, bool>(&mut conn)
            .await?;
        Ok(result)
    }

    async fn release_init_lock(&self, sponsor_address: SuiAddress) -> anyhow::Result<()> {
        debug!("Releasing the init lock.");
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        ScriptManager::release_init_lock_script()
            .arg(sponsor_str)
            .invoke_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    async fn check_health(&self) -> anyhow::Result<()> {
        let mut conn = self.conn_manager.clone();
        redis::cmd("PING").query_async(&mut conn).await?;
        Ok(())
    }

    #[cfg(test)]
    async fn flush_db(&self) {
        let mut conn = self.conn_manager.clone();
        redis::cmd("FLUSHDB")
            .query_async::<_, String>(&mut conn)
            .await
            .unwrap();
    }

    async fn get_available_coin_count(&self, sponsor_address: SuiAddress) -> anyhow::Result<usize> {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        let count = ScriptManager::get_available_coin_count_script()
            .arg(sponsor_str)
            .invoke_async::<_, usize>(&mut conn)
            .await?;
        Ok(count)
    }

    async fn get_available_coin_total_balance(&self, sponsor_address: SuiAddress) -> u64 {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        ScriptManager::get_available_coin_total_balance_script()
            .arg(sponsor_str)
            .invoke_async::<_, u64>(&mut conn)
            .await
            .unwrap()
    }

    #[cfg(test)]
    async fn get_reserved_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        let sponsor_str = sponsor_address.to_string();
        let mut conn = self.conn_manager.clone();
        ScriptManager::get_reserved_coin_count_script()
            .arg(sponsor_str)
            .invoke_async::<_, usize>(&mut conn)
            .await
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use sui_types::base_types::{random_object_ref, SuiAddress};

    use crate::{
        metrics::StorageMetrics,
        storage::{redis::RedisStorage, Storage},
        types::GasCoin,
    };

    #[tokio::test]
    async fn test_init_coin_stats_at_startup() {
        let storage = setup_storage().await;
        let sponsor = SuiAddress::ZERO;
        storage
            .add_new_coins(
                sponsor,
                vec![
                    GasCoin {
                        balance: 100,
                        object_ref: random_object_ref(),
                    },
                    GasCoin {
                        balance: 200,
                        object_ref: random_object_ref(),
                    },
                ],
            )
            .await
            .unwrap();
        let (coin_count, total_balance) =
            storage.init_coin_stats_at_startup(sponsor).await.unwrap();
        assert_eq!(coin_count, 2);
        assert_eq!(total_balance, 300);
    }

    #[tokio::test]
    async fn test_add_new_coins() {
        let sponsor = SuiAddress::ZERO;
        let storage = setup_storage().await;
        storage
            .add_new_coins(
                sponsor,
                vec![
                    GasCoin {
                        balance: 100,
                        object_ref: random_object_ref(),
                    },
                    GasCoin {
                        balance: 200,
                        object_ref: random_object_ref(),
                    },
                ],
            )
            .await
            .unwrap();
        let coin_count = storage.get_available_coin_count(sponsor).await.unwrap();
        assert_eq!(coin_count, 2);
        let total_balance = storage.get_available_coin_total_balance(sponsor).await;
        assert_eq!(total_balance, 300);
        storage
            .add_new_coins(
                sponsor,
                vec![
                    GasCoin {
                        balance: 300,
                        object_ref: random_object_ref(),
                    },
                    GasCoin {
                        balance: 400,
                        object_ref: random_object_ref(),
                    },
                ],
            )
            .await
            .unwrap();
        let coin_count = storage.get_available_coin_count(sponsor).await.unwrap();
        assert_eq!(coin_count, 4);
        let total_balance = storage.get_available_coin_total_balance(sponsor).await;
        assert_eq!(total_balance, 1000);
    }

    async fn setup_storage() -> RedisStorage {
        let storage =
            RedisStorage::new("redis://127.0.0.1:6379", StorageMetrics::new_for_testing()).await;
        storage.flush_db().await;
        let (coin_count, total_balance) = storage
            .init_coin_stats_at_startup(SuiAddress::ZERO)
            .await
            .unwrap();
        assert_eq!(coin_count, 0);
        assert_eq!(total_balance, 0);
        storage
    }
}
