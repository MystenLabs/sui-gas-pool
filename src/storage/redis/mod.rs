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

        let expiration_time = Utc::now()
            .add(Duration::from_millis(reserved_duration_ms))
            .timestamp_millis() as u64;
        let mut conn = self.conn_manager.clone();
        let (reservation_id, coins): (ReservationID, Vec<String>) =
            ScriptManager::reserve_gas_coins_script()
                .arg(sponsor_address.to_string())
                .arg(target_budget)
                .arg(expiration_time)
                .invoke_async(&mut conn)
                .await?;
        assert!(!coins.is_empty());
        let gas_coins = coins
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

        self.metrics.num_successful_reserve_gas_coins_requests.inc();
        Ok((reservation_id, gas_coins))
    }

    async fn ready_for_execution(
        &self,
        sponsor_address: SuiAddress,
        reservation_id: ReservationID,
    ) -> anyhow::Result<()> {
        self.metrics.num_ready_for_execution_requests.inc();

        let mut conn = self.conn_manager.clone();
        ScriptManager::ready_for_execution_script()
            .arg(sponsor_address.to_string())
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
        ScriptManager::add_new_coins_script()
            .arg(sponsor_address.to_string())
            .arg(serde_json::to_string(&formatted_coins)?)
            .invoke_async(&mut conn)
            .await?;

        self.metrics.num_successful_add_new_coins_requests.inc();
        Ok(())
    }

    async fn expire_coins(&self, sponsor_address: SuiAddress) -> anyhow::Result<Vec<ObjectID>> {
        self.metrics.num_expire_coins_requests.inc();

        let now = Utc::now().timestamp_millis() as u64;
        let mut conn = self.conn_manager.clone();
        let expired_coin_strings: Vec<String> = ScriptManager::expire_coins_script()
            .arg(sponsor_address.to_string())
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

    async fn remove_all_available_coins(&self, sponsor_address: SuiAddress) -> anyhow::Result<()> {
        let mut conn = self.conn_manager.clone();
        ScriptManager::remove_all_available_coins_script()
            .arg(sponsor_address.to_string())
            .invoke_async::<_, ()>(&mut conn)
            .await?;
        Ok(())
    }

    async fn is_initialized(&self, sponsor_address: SuiAddress) -> anyhow::Result<bool> {
        let mut conn = self.conn_manager.clone();
        let result = ScriptManager::get_is_initialized_script()
            .arg(sponsor_address.to_string())
            .invoke_async::<_, bool>(&mut conn)
            .await?;
        Ok(result)
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
        let mut conn = self.conn_manager.clone();
        let count = ScriptManager::get_available_coin_count_script()
            .arg(sponsor_address.to_string())
            .invoke_async::<_, usize>(&mut conn)
            .await?;
        Ok(count)
    }

    #[cfg(test)]
    async fn get_available_coin_total_balance(&self, sponsor_address: SuiAddress) -> u64 {
        let mut conn = self.conn_manager.clone();
        ScriptManager::get_available_coin_total_balance_script()
            .arg(sponsor_address.to_string())
            .invoke_async::<_, u64>(&mut conn)
            .await
            .unwrap()
    }

    #[cfg(test)]
    async fn get_reserved_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        let mut conn = self.conn_manager.clone();
        ScriptManager::get_reserved_coin_count_script()
            .arg(sponsor_address.to_string())
            .invoke_async::<_, usize>(&mut conn)
            .await
            .unwrap()
    }
}
