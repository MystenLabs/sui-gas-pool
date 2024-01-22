// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

mod script_manager;

use crate::metrics::StorageMetrics;
use crate::storage::redis::script_manager::ScriptManager;
use crate::storage::Storage;
use crate::types::{GasCoin, ReservationID};
use anyhow::bail;
use chrono::Utc;
use parking_lot::Mutex;
use std::ops::Add;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use sui_types::base_types::{ObjectDigest, ObjectID, SequenceNumber, SuiAddress};

pub struct RedisStorage {
    _client: redis::Client,
    connection: Mutex<redis::Connection>,
    metrics: Arc<StorageMetrics>,
}

impl RedisStorage {
    pub fn new(redis_url: &str, metrics: Arc<StorageMetrics>) -> Self {
        let _client = redis::Client::open(redis_url).unwrap();
        let connection = Mutex::new(_client.get_connection().unwrap());
        Self {
            _client,
            connection,
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
        let (reservation_id, balances, object_ids, versions, digests): (
            ReservationID,
            Vec<u64>,
            Vec<String>,
            Vec<u64>,
            Vec<String>,
        ) = ScriptManager::reserve_gas_coins_script()
            .arg(sponsor_address.to_string())
            .arg(target_budget)
            .arg(expiration_time)
            .invoke(&mut self.connection.lock())?;
        if balances.is_empty() {
            bail!("No coins available for reservation");
        }
        assert_eq!(balances.len(), object_ids.len());
        assert_eq!(object_ids.len(), versions.len());
        assert_eq!(versions.len(), digests.len());
        let gas_coins = (0..balances.len())
            .map(|i| {
                let object_id = ObjectID::from_str(object_ids[i].as_str()).unwrap();
                let version = SequenceNumber::from(versions[i]);
                let digest = ObjectDigest::from_str(digests[i].as_str()).unwrap();
                let balance = balances[i];
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

        let result: bool = ScriptManager::ready_for_execution_script()
            .arg(sponsor_address.to_string())
            .arg(reservation_id)
            .invoke(&mut self.connection.lock())?;
        if !result {
            bail!(
                "Reservation {} not found, likely already expired",
                reservation_id
            );
        }

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

        let mut balances = Vec::with_capacity(new_coins.len());
        let mut object_ids = Vec::with_capacity(new_coins.len());
        let mut versions = Vec::with_capacity(new_coins.len());
        let mut digests = Vec::with_capacity(new_coins.len());
        for coin in new_coins {
            balances.push(coin.balance);
            object_ids.push(coin.object_ref.0.to_string());
            versions.push(coin.object_ref.1.value());
            digests.push(coin.object_ref.2.to_string());
        }
        ScriptManager::add_new_coins_script()
            .arg(sponsor_address.to_string())
            .arg(serde_json::to_string(&balances)?)
            .arg(serde_json::to_string(&object_ids)?)
            .arg(serde_json::to_string(&versions)?)
            .arg(serde_json::to_string(&digests)?)
            .invoke(&mut self.connection.lock())?;

        self.metrics.num_successful_add_new_coins_requests.inc();
        Ok(())
    }

    async fn expire_coins(&self, sponsor_address: SuiAddress) -> anyhow::Result<Vec<ObjectID>> {
        self.metrics.num_expire_coins_requests.inc();

        let now = Utc::now().timestamp_millis() as u64;
        let expired_coin_strings: Vec<String> = ScriptManager::expire_coins_script()
            .arg(sponsor_address.to_string())
            .arg(now)
            .invoke(&mut self.connection.lock())?;
        // The script returns a list of comma separated coin ids.
        let expired_coin_ids = expired_coin_strings
            .iter()
            .flat_map(|s| s.split(',').map(|id| ObjectID::from_str(id).unwrap()))
            .collect();

        self.metrics.num_successful_expire_coins_requests.inc();
        Ok(expired_coin_ids)
    }

    async fn check_health(&self) -> anyhow::Result<()> {
        redis::cmd("PING").query::<String>(&mut self.connection.lock())?;
        Ok(())
    }

    #[cfg(test)]
    async fn flush_db(&self) {
        redis::cmd("FLUSHDB")
            .query::<String>(&mut self.connection.lock())
            .unwrap();
    }

    async fn get_available_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        ScriptManager::get_available_coin_count_script()
            .arg(sponsor_address.to_string())
            .invoke::<usize>(&mut self.connection.lock())
            .unwrap()
    }

    #[cfg(test)]
    async fn get_available_coin_total_balance(&self, sponsor_address: SuiAddress) -> u64 {
        ScriptManager::get_available_coin_total_balance_script()
            .arg(sponsor_address.to_string())
            .invoke::<u64>(&mut self.connection.lock())
            .unwrap()
    }

    #[cfg(test)]
    async fn get_reserved_coin_count(&self, sponsor_address: SuiAddress) -> usize {
        ScriptManager::get_reserved_coin_count_script()
            .arg(sponsor_address.to_string())
            .invoke::<usize>(&mut self.connection.lock())
            .unwrap()
    }
}
