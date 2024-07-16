// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod gas_pool_core;
mod gas_usage_cap;

#[cfg(test)]
mod tests {
    use crate::test_env::{create_test_transaction, start_gas_station};
    use shared_crypto::intent::{Intent, IntentMessage};
    use std::time::Duration;
    use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
    use sui_types::{
        crypto::{get_account_key_pair, Signature},
        gas_coin::MIST_PER_SUI,
        programmable_transaction_builder::ProgrammableTransactionBuilder,
        transaction::{TransactionData, TransactionKind},
    };

    #[tokio::test]
    async fn test_station_reserve_gas() {
        let (_test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI; 10], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        let (sponsor1, _res_id1, gas_coins) = station
            .reserve_gas(MIST_PER_SUI * 3, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 3);
        assert_eq!(station.query_pool_available_coin_count().await, 7);
        let (sponsor2, _res_id2, gas_coins) = station
            .reserve_gas(MIST_PER_SUI * 7, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 7);
        assert_eq!(sponsor1, sponsor2);
        assert_eq!(station.query_pool_available_coin_count().await, 0);
        assert!(station
            .reserve_gas(1, Duration::from_secs(10))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_e2e_gas_station_flow() {
        let (test_cluster, container) = start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        assert!(station
            .reserve_gas(MIST_PER_SUI + 1, Duration::from_secs(10))
            .await
            .is_err());

        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 1);
        assert_eq!(station.query_pool_available_coin_count().await, 0);
        assert!(station
            .reserve_gas(1, Duration::from_secs(10))
            .await
            .is_err());

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        let response = station
            .execute_transaction(reservation_id, tx_data, user_sig)
            .await
            .unwrap();
        assert!(response.effects.clone().unwrap().status().is_ok());
        assert_eq!(station.query_pool_available_coin_count().await, 1);
    }

    #[tokio::test]
    async fn test_invalid_transaction() {
        telemetry_subscribers::init_for_testing();
        let (_test_cluster, container) = start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();
        let (sender, keypair) = get_account_key_pair();
        let tx_kind = TransactionKind::programmable(ProgrammableTransactionBuilder::new().finish());
        let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            tx_kind, sender, gas_coins, 1, 1, sponsor,
        );
        let user_sig = Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );
        let result = station
            .execute_transaction(reservation_id, tx_data, user_sig.into())
            .await;
        println!("{:?}", result);
        assert!(result.is_err());
        assert_eq!(station.query_pool_available_coin_count().await, 1);
    }

    #[tokio::test]
    async fn test_coin_expiration() {
        telemetry_subscribers::init_for_testing();
        let (test_cluster, container) = start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 1);
        assert_eq!(station.query_pool_available_coin_count().await, 0);
        assert!(station
            .reserve_gas(1, Duration::from_secs(1))
            .await
            .is_err());
        // Sleep a little longer to give it enough time to expire.
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(station.query_pool_available_coin_count().await, 1);
        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        assert!(station
            .execute_transaction(reservation_id, tx_data, user_sig)
            .await
            .is_err());
        station
            .reserve_gas(1, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[ignore]
    #[tokio::test]
    async fn test_incomplete_gas_usage() {
        let (test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI; 10], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI * 3, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 3);

        // Remove one gas object from the reserved list and only use the two.
        let mut incomplete_gas_coins = gas_coins.clone();
        incomplete_gas_coins.pop().unwrap();
        let (tx_data, user_sig) =
            create_test_transaction(&test_cluster, sponsor, incomplete_gas_coins).await;
        // It should fail because it's inconsistent with the reservation.
        assert!(station
            .execute_transaction(reservation_id, tx_data, user_sig)
            .await
            .is_err());

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        let effects = station
            .execute_transaction(reservation_id, tx_data, user_sig)
            .await
            .unwrap();
        assert!(effects.effects.clone().unwrap().status().is_ok());
    }

    #[ignore]
    #[tokio::test]
    async fn test_mixed_up_gas_coins() {
        let (test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI; 10], MIST_PER_SUI).await;
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id1, gas_coins1) = station
            .reserve_gas(MIST_PER_SUI * 3, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins1.len(), 3);
        let (_, _res_id2, gas_coins2) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins2.len(), 1);

        // Mix up gas coins from two reservations.
        let mut mixed_up_gas_coins = gas_coins1.clone();
        mixed_up_gas_coins[0] = gas_coins2[0];
        let (tx_data, user_sig) =
            create_test_transaction(&test_cluster, sponsor, mixed_up_gas_coins).await;
        assert!(station
            .execute_transaction(reservation_id1, tx_data, user_sig)
            .await
            .is_err());

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins1).await;
        let response = station
            .execute_transaction(reservation_id1, tx_data, user_sig)
            .await
            .unwrap();
        assert!(response.effects.unwrap().status().is_ok());
    }
}
