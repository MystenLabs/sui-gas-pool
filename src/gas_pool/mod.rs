// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod gas_pool_core;
mod gas_usage_cap;

#[cfg(test)]
mod tests {
    use crate::test_env::{
        create_pay_sui_transaction_same_sender_as_sponsor, create_test_transaction,
        create_test_transaction_with_same_sender_as_sponsor, start_gas_station,
        start_gas_station_with_cluster, start_sui_cluster,
    };
    use shared_crypto::intent::{Intent, IntentMessage};
    use std::time::Duration;
    use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
    use sui_types::{
        SUI_FRAMEWORK_PACKAGE_ID,
        coin::{
            COIN_MODULE_NAME, PAY_MODULE_NAME, PAY_SPLIT_N_FUNC_NAME, REDEEM_FUNDS_FUNC_NAME,
            SEND_FUNDS_FUNC_NAME,
        },
        crypto::{Signature, get_account_key_pair},
        effects::TransactionEffectsAPI,
        gas_coin::{GAS, MIST_PER_SUI},
        programmable_transaction_builder::ProgrammableTransactionBuilder,
        transaction::{
            Argument, CallArg, Command, FundsWithdrawalArg, ObjectArg, TransactionData,
            TransactionKind,
        },
    };

    const TEST_ADVANCED_FAUCET_MODE: bool = false;

    #[tokio::test]
    async fn test_station_reserve_gas() {
        let (_test_cluster, container) = start_gas_station(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await
        .unwrap();
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
        assert!(
            station
                .reserve_gas(1, Duration::from_secs(10))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_e2e_gas_station_flow() {
        let (test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();
        let station = container.get_gas_pool_arc();
        assert!(
            station
                .reserve_gas(MIST_PER_SUI + 1, Duration::from_secs(10))
                .await
                .is_err()
        );

        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 1);
        assert_eq!(station.query_pool_available_coin_count().await, 0);
        assert!(
            station
                .reserve_gas(1, Duration::from_secs(10))
                .await
                .is_err()
        );

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        let tx_block_response = station
            .execute_transaction(reservation_id, tx_data, user_sig, None)
            .await
            .unwrap();
        assert!(tx_block_response.effects.unwrap().status().is_ok());
        assert_eq!(station.query_pool_available_coin_count().await, 1);
    }

    #[tokio::test]
    async fn test_invalid_transaction() {
        telemetry_subscribers::init_for_testing();
        let (_test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();
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
            .execute_transaction(reservation_id, tx_data, user_sig.into(), None)
            .await;
        println!("{:?}", result);
        assert!(result.is_err());
        assert_eq!(station.query_pool_available_coin_count().await, 1);
    }

    #[tokio::test]
    async fn test_rejects_same_sender_as_sponsor_when_not_in_advanced_faucet_mode() {
        let (_test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();

        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();

        let mut ptb = ProgrammableTransactionBuilder::new();
        ptb.input(CallArg::Object(ObjectArg::ImmOrOwnedObject(gas_coins[0])))
            .unwrap();

        let tx_kind = TransactionKind::programmable(ptb.finish());
        let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            tx_kind, sponsor, gas_coins, 1, 1, sponsor,
        );

        let (_, keypair) = get_account_key_pair();
        let user_sig = Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );

        let err = station
            .execute_transaction(reservation_id, tx_data, user_sig.into(), None)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("Sender cannot match sponsor"));
    }

    #[tokio::test]
    async fn test_rejects_gas_coin_misuse_when_not_in_advanced_faucet_mode() {
        let (_test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();

        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();

        let (sender, keypair) = get_account_key_pair();
        let mut ptb = ProgrammableTransactionBuilder::new();
        let split_count = ptb.pure(1u64).unwrap();
        ptb.programmable_move_call(
            SUI_FRAMEWORK_PACKAGE_ID,
            PAY_MODULE_NAME.into(),
            PAY_SPLIT_N_FUNC_NAME.into(),
            vec![GAS::type_tag()],
            vec![Argument::GasCoin, split_count],
        );

        let tx_kind = TransactionKind::programmable(ptb.finish());
        let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            tx_kind, sender, gas_coins, 1, 1, sponsor,
        );

        let user_sig = Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );

        let err = station
            .execute_transaction(reservation_id, tx_data, user_sig.into(), None)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("Gas coin can only be used to pay gas"));
    }

    #[tokio::test]
    async fn test_transaction_with_address_balance_reservation() {
        let (test_cluster, container) = start_gas_station(
            vec![MIST_PER_SUI; 5],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await
        .unwrap();

        let gas_price = test_cluster.get_reference_gas_price().await;
        let station = container.get_gas_pool_arc();
        let sender = test_cluster.get_address_1();

        {
            // (1). Convert some of the sender's gas coin into an address balance.
            let sender_gas = test_cluster
                .wallet
                .get_one_gas_object_owned_by_address(sender)
                .await
                .unwrap()
                .unwrap();

            let mut ptb = ProgrammableTransactionBuilder::new();

            let amount_arg = ptb.pure(MIST_PER_SUI).unwrap();
            let coin_arg = ptb.command(Command::SplitCoins(Argument::GasCoin, vec![amount_arg]));
            let sender_arg = ptb.pure(sender).unwrap();

            ptb.programmable_move_call(
                SUI_FRAMEWORK_PACKAGE_ID,
                COIN_MODULE_NAME.to_owned(),
                SEND_FUNDS_FUNC_NAME.to_owned(),
                vec![GAS::type_tag()],
                vec![coin_arg, sender_arg],
            );

            let tx_kind = TransactionKind::programmable(ptb.finish());
            let tx_data =
                TransactionData::new(tx_kind, sender, sender_gas, MIST_PER_SUI / 100, gas_price);

            let response = test_cluster.sign_and_execute_transaction(&tx_data).await;
            assert!(response.effects.status().is_ok());
        }

        {
            // (2). Make use of the address balance in a new transaction with a new reservation.
            let (sponsor, reservation_id, gas_coins) = station
                .reserve_gas(MIST_PER_SUI * 5, Duration::from_secs(10))
                .await
                .unwrap();

            let mut ptb = ProgrammableTransactionBuilder::new();
            let sender_arg = ptb.pure(sender).unwrap();
            let withdrawal = ptb
                .funds_withdrawal(FundsWithdrawalArg::balance_from_sender(
                    MIST_PER_SUI,
                    GAS::type_tag(),
                ))
                .unwrap();

            let coin = ptb.programmable_move_call(
                SUI_FRAMEWORK_PACKAGE_ID,
                COIN_MODULE_NAME.to_owned(),
                REDEEM_FUNDS_FUNC_NAME.to_owned(),
                vec![GAS::type_tag()],
                vec![withdrawal],
            );

            ptb.programmable_move_call(
                SUI_FRAMEWORK_PACKAGE_ID,
                COIN_MODULE_NAME.to_owned(),
                SEND_FUNDS_FUNC_NAME.to_owned(),
                vec![GAS::type_tag()],
                vec![coin, sender_arg],
            );

            let tx_kind = TransactionKind::programmable(ptb.finish());
            let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
                tx_kind,
                sender,
                gas_coins,
                MIST_PER_SUI / 100,
                gas_price,
                sponsor,
            );

            let user_sig = test_cluster
                .sign_transaction(&tx_data)
                .await
                .tx_signatures()
                .first()
                .cloned()
                .unwrap();

            let response = station
                .execute_transaction(reservation_id, tx_data, user_sig, None)
                .await
                .unwrap();

            assert!(response.effects.unwrap().status().is_ok());
            assert_eq!(station.query_pool_available_coin_count().await, 1);
        }
    }

    #[tokio::test]
    async fn test_rejects_sponsor_funds_withdrawal_when_not_in_advanced_faucet_mode() {
        let (test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();

        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();

        let (sender, keypair) = get_account_key_pair();
        let mut ptb = ProgrammableTransactionBuilder::new();

        let sender_arg = ptb.pure(sender).unwrap();
        let withdrawal = ptb
            .funds_withdrawal(FundsWithdrawalArg::balance_from_sponsor(
                MIST_PER_SUI,
                GAS::type_tag(),
            ))
            .unwrap();

        let coin = ptb.programmable_move_call(
            SUI_FRAMEWORK_PACKAGE_ID,
            COIN_MODULE_NAME.to_owned(),
            REDEEM_FUNDS_FUNC_NAME.to_owned(),
            vec![GAS::type_tag()],
            vec![withdrawal],
        );

        ptb.programmable_move_call(
            SUI_FRAMEWORK_PACKAGE_ID,
            COIN_MODULE_NAME.to_owned(),
            SEND_FUNDS_FUNC_NAME.to_owned(),
            vec![GAS::type_tag()],
            vec![coin, sender_arg],
        );

        let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            TransactionKind::programmable(ptb.finish()),
            sender,
            gas_coins,
            MIST_PER_SUI / 100,
            test_cluster.get_reference_gas_price().await,
            sponsor,
        );

        let user_sig = Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );

        let err = station
            .execute_transaction(reservation_id, tx_data, user_sig.into(), None)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("Funds withdrawal from sponsor is not supported"));
    }

    #[tokio::test]
    async fn test_rejects_sponsor_funds_withdrawal_in_advanced_faucet_mode() {
        let (mut test_cluster, signer, keypair) = start_sui_cluster(vec![MIST_PER_SUI]).await;
        let (_, container) =
            start_gas_station_with_cluster(&mut test_cluster, signer, MIST_PER_SUI, true)
                .await
                .unwrap();

        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(10))
            .await
            .unwrap();

        let mut ptb = ProgrammableTransactionBuilder::new();

        let sponsor_arg = ptb.pure(sponsor).unwrap();
        let withdrawal = ptb
            .funds_withdrawal(FundsWithdrawalArg::balance_from_sponsor(
                MIST_PER_SUI,
                GAS::type_tag(),
            ))
            .unwrap();

        let coin = ptb.programmable_move_call(
            SUI_FRAMEWORK_PACKAGE_ID,
            COIN_MODULE_NAME.to_owned(),
            REDEEM_FUNDS_FUNC_NAME.to_owned(),
            vec![GAS::type_tag()],
            vec![withdrawal],
        );

        ptb.programmable_move_call(
            SUI_FRAMEWORK_PACKAGE_ID,
            COIN_MODULE_NAME.to_owned(),
            SEND_FUNDS_FUNC_NAME.to_owned(),
            vec![GAS::type_tag()],
            vec![coin, sponsor_arg],
        );

        let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            TransactionKind::programmable(ptb.finish()),
            sponsor,
            gas_coins,
            MIST_PER_SUI / 100,
            test_cluster.get_reference_gas_price().await,
            sponsor,
        );

        let user_sig = Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );

        let err = station
            .execute_transaction(reservation_id, tx_data, user_sig.into(), None)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("Funds withdrawal from sponsor is not supported"));
    }

    #[tokio::test]
    async fn test_coin_expiration() {
        telemetry_subscribers::init_for_testing();
        let (test_cluster, container) =
            start_gas_station(vec![MIST_PER_SUI], MIST_PER_SUI, TEST_ADVANCED_FAUCET_MODE)
                .await
                .unwrap();
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id, gas_coins) = station
            .reserve_gas(MIST_PER_SUI, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(gas_coins.len(), 1);
        assert_eq!(station.query_pool_available_coin_count().await, 0);
        assert!(
            station
                .reserve_gas(1, Duration::from_secs(1))
                .await
                .is_err()
        );
        // Sleep a little longer to give it enough time to expire.
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(station.query_pool_available_coin_count().await, 1);
        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        assert!(
            station
                .execute_transaction(reservation_id, tx_data, user_sig, None)
                .await
                .is_err()
        );
        station
            .reserve_gas(1, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[ignore]
    #[tokio::test]
    async fn test_incomplete_gas_usage() {
        let (test_cluster, container) = start_gas_station(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await
        .unwrap();
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
        assert!(
            station
                .execute_transaction(reservation_id, tx_data, user_sig, None)
                .await
                .is_err()
        );

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        let tx_block_response = station
            .execute_transaction(reservation_id, tx_data, user_sig, None)
            .await
            .unwrap();
        assert!(tx_block_response.effects.unwrap().status().is_ok());
    }

    #[ignore]
    #[tokio::test]
    async fn test_mixed_up_gas_coins() {
        let (test_cluster, container) = start_gas_station(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await
        .unwrap();
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
        assert!(
            station
                .execute_transaction(reservation_id1, tx_data, user_sig, None)
                .await
                .is_err()
        );

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins1).await;
        let tx_block_response = station
            .execute_transaction(reservation_id1, tx_data, user_sig, None)
            .await
            .unwrap();
        assert!(tx_block_response.effects.unwrap().status().is_ok());
    }

    // #[ignore]
    #[tokio::test]
    async fn test_advanced_faucet_mode() {
        // In advanced faucet mode, the sponsor and sender have to be the same, and the signer
        // needs to be the sender.

        // Create a test cluster with advanced faucet mode enabled.
        let (mut test_cluster, signer, keypair) = start_sui_cluster(vec![MIST_PER_SUI; 20]).await;
        let (_, container) = start_gas_station_with_cluster(
            &mut test_cluster,
            signer,
            MIST_PER_SUI,
            true, /* advanced_faucet_mode */
        )
        .await
        .unwrap();
        let station = container.get_gas_pool_arc();
        let (sponsor, reservation_id1, gas_coins1) = station
            .reserve_gas(MIST_PER_SUI * 3, Duration::from_secs(10))
            .await
            .unwrap();
        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins1).await;
        let tx = station
            .execute_transaction(reservation_id1, tx_data, user_sig, None)
            .await;
        assert!(tx.is_err());
        assert!(
            tx.unwrap_err()
                .to_string()
                .contains("Expected that the transaction signer is the same as the sender")
        );

        let (sponsor, reservation_id2, gas_coins2) = station
            .reserve_gas(MIST_PER_SUI * 5, Duration::from_secs(10))
            .await
            .unwrap();
        let (tx_data, user_sig) = create_test_transaction_with_same_sender_as_sponsor(
            &mut test_cluster,
            sponsor,
            keypair.copy(),
            gas_coins2,
        )
        .await;

        let tx_block_response = station
            .execute_transaction(reservation_id2, tx_data, user_sig, None)
            .await;

        assert!(tx_block_response.is_ok(), "{:?}", tx_block_response);
        assert!(tx_block_response.unwrap().effects.unwrap().status().is_ok());

        let (sponsor, reservation_id3, gas_coins3) = station
            .reserve_gas(MIST_PER_SUI * 5, Duration::from_secs(10))
            .await
            .unwrap();
        let (tx_data, user_sig) = create_pay_sui_transaction_same_sender_as_sponsor(
            &mut test_cluster,
            sponsor,
            keypair,
            gas_coins3,
        )
        .await;
        let tx_block_response = station
            .execute_transaction(reservation_id3, tx_data, user_sig, None)
            .await;

        assert!(tx_block_response.is_ok());
        assert!(tx_block_response.unwrap().effects.unwrap().status().is_ok());
    }
}
