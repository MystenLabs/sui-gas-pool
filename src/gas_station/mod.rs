// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::types::GaslessTransaction;
use shared_crypto::intent::IntentMessage;
use std::time::Duration;
use sui_types::base_types::ObjectID;
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

pub mod simple_gas_station;

#[async_trait::async_trait]
pub trait GasStation: Send + Sync {
    // TODO: To increase throughput we could support batched reservations.
    async fn reserve_gas(
        &self,
        tx: GaslessTransaction,
        duration: Duration,
    ) -> anyhow::Result<(IntentMessage<TransactionData>, GenericSignature)>;

    /// Returns the number of objects successfully released.
    async fn release_gas(&self, gas_coins: Vec<ObjectID>) -> usize;
}

#[cfg(test)]
mod tests {
    use crate::gas_station::GasStation;
    use crate::test_env::start_gas_station;
    use crate::types::GaslessTransaction;
    use std::time::Duration;
    use sui_types::gas_coin::MIST_PER_SUI;
    use sui_types::transaction::TransactionDataAPI;

    #[tokio::test]
    async fn test_basic_gas_station_flow() {
        let (_test_cluster, station) = start_gas_station(vec![MIST_PER_SUI]).await;
        let mut tx = GaslessTransaction::default();
        tx.budget = MIST_PER_SUI + 1;
        assert!(station
            .reserve_gas(tx.clone(), Duration::from_secs(1))
            .await
            .is_err());

        tx.budget = MIST_PER_SUI;
        let (tx_data, sig) = station
            .reserve_gas(tx.clone(), Duration::from_secs(1))
            .await
            .unwrap();
        tx.check_consistent_with(&tx_data, &sig).unwrap();
        let gas_object = tx_data.value.gas_data().payment[0];
        assert_eq!(station.release_gas(vec![gas_object.0]).await, 1);
    }
}
