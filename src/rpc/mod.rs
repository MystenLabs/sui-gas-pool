// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod client;
mod rpc_types;
mod server;

pub use server::GasPoolServer;

#[cfg(test)]
mod tests {
    use crate::AUTH_ENV_NAME;
    use crate::rpc::server::MAX_INPUT_OBJECTS;
    use crate::test_env::{create_test_transaction, start_rpc_server_for_testing};
    use shared_crypto::intent::{Intent, IntentMessage};
    use sui_json_rpc_types::{SuiTransactionBlockEffectsAPI, SuiTransactionBlockResponseOptions};
    use sui_types::base_types::{ObjectID, ObjectRef, SequenceNumber, SuiAddress};
    use sui_types::crypto::get_account_key_pair;
    use sui_types::digests::ObjectDigest;
    use sui_types::gas_coin::MIST_PER_SUI;
    use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
    use sui_types::transaction::{
        CallArg, ObjectArg, TransactionData, TransactionDataAPI, TransactionKind,
    };

    const TEST_ADVANCED_FAUCET_MODE: bool = false;

    #[tokio::test]
    async fn test_basic_rpc_flow() {
        let (test_cluster, _container, server) = start_rpc_server_for_testing(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await;
        let client = server.get_local_client();
        client.health().await.unwrap();

        let (sponsor, reservation_id, gas_coins) =
            client.reserve_gas(MIST_PER_SUI, 10).await.unwrap();
        assert_eq!(gas_coins.len(), 1);

        // We can no longer request all balance given one is loaned out above.
        assert!(client.reserve_gas(MIST_PER_SUI * 10, 10).await.is_err());

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        let tx_response = client
            .execute_tx(reservation_id, &tx_data, &user_sig, None)
            .await
            .unwrap();
        assert!(tx_response.effects.unwrap().status().is_ok());
        assert!(tx_response.tx_block_response.is_none());
    }

    #[tokio::test]
    async fn test_rpc_flow_with_options() {
        let (test_cluster, _container, server) = start_rpc_server_for_testing(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await;
        let client = server.get_local_client();

        let (sponsor, reservation_id, gas_coins) =
            client.reserve_gas(MIST_PER_SUI, 10).await.unwrap();
        assert_eq!(gas_coins.len(), 1);

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;

        // Testing with object changes that is not part of the default response
        let tx_response = client
            .execute_tx(
                reservation_id,
                &tx_data,
                &user_sig,
                Some(SuiTransactionBlockResponseOptions::new().with_object_changes()),
            )
            .await
            .unwrap();

        // Should not observe effects as this field gets populated only when no options are passed.
        assert!(tx_response.effects.is_none());
        assert!(tx_response.tx_block_response.is_some());
        // Should observe the requested object_changes
        assert!(
            tx_response
                .tx_block_response
                .as_ref()
                .unwrap()
                .object_changes
                .is_some()
        );
        // Should not observe effects as we did not request for them, even though the service utilizes them internally.
        assert!(
            tx_response
                .tx_block_response
                .as_ref()
                .unwrap()
                .effects
                .is_none()
        );
        // Should not observe balance_changes as we did not request for them, even though the service utilizes them internally.
        assert!(
            tx_response
                .tx_block_response
                .as_ref()
                .unwrap()
                .balance_changes
                .is_none()
        );

        let (sponsor, reservation_id, gas_coins) =
            client.reserve_gas(MIST_PER_SUI, 10).await.unwrap();
        assert_eq!(gas_coins.len(), 1);

        let (tx_data, user_sig) = create_test_transaction(&test_cluster, sponsor, gas_coins).await;
        // Testing with effects that is part of the required options used internally
        let tx_response = client
            .execute_tx(
                reservation_id,
                &tx_data,
                &user_sig,
                Some(SuiTransactionBlockResponseOptions::new().with_effects()),
            )
            .await
            .unwrap();

        // Should not observe effects as this field gets populated only when no options are passed.
        assert!(tx_response.effects.is_none());
        assert!(tx_response.tx_block_response.is_some());
        // Should observe effects within tx_block_response as we requested for them.
        assert!(
            tx_response
                .tx_block_response
                .as_ref()
                .unwrap()
                .effects
                .is_some()
        );
        // Should not observe balance_changes as we did not request for them, even though the service utilizes them internally.
        assert!(
            tx_response
                .tx_block_response
                .as_ref()
                .unwrap()
                .balance_changes
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_invalid_auth() {
        let (_test_cluster, _container, server) = start_rpc_server_for_testing(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await;

        let client = server.get_local_client();
        client.health().await.unwrap();

        let (_sponsor, _res_id, gas_coins) = client.reserve_gas(MIST_PER_SUI, 10).await.unwrap();
        assert_eq!(gas_coins.len(), 1);

        // Change the auth secret used in the client.
        unsafe {
            std::env::set_var(AUTH_ENV_NAME, "b");
        }

        assert!(client.reserve_gas(MIST_PER_SUI, 10).await.is_err());
    }

    #[tokio::test]
    async fn test_debug_health_check() {
        let (_test_cluster, _container, server) = start_rpc_server_for_testing(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await;

        let client = server.get_local_client();
        client.debug_health_check().await.unwrap();
    }

    fn create_transaction_with_many_input_objects(
        sender: sui_types::base_types::SuiAddress,
        sponsor: sui_types::base_types::SuiAddress,
        gas_coins: Vec<ObjectRef>,
        num_input_objects: usize,
    ) -> (TransactionData, sui_types::signature::GenericSignature) {
        let mut ptb = ProgrammableTransactionBuilder::new();

        // Add many input objects to exceed the limit
        for i in 0..num_input_objects {
            let object_id = ObjectID::from_bytes(&[i as u8; 32]).unwrap();
            let version = SequenceNumber::from_u64(1);
            let digest = ObjectDigest::random();
            ptb.input(CallArg::Object(ObjectArg::ImmOrOwnedObject((
                object_id, version, digest,
            ))))
            .unwrap();
        }

        let pt = ptb.finish();
        let mut tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
            TransactionKind::ProgrammableTransaction(pt),
            sender,
            gas_coins.clone(),
            10000000, // gas_budget
            1000,     // gas_price
            sponsor,
        );
        tx_data.gas_data_mut().payment = gas_coins;
        tx_data.gas_data_mut().owner = sponsor;

        // Create a dummy signature
        let (_, keypair) = get_account_key_pair();
        let signature = sui_types::crypto::Signature::new_secure(
            &IntentMessage::new(Intent::sui_transaction(), &tx_data),
            &keypair,
        );

        (
            tx_data,
            sui_types::signature::GenericSignature::Signature(signature),
        )
    }

    #[tokio::test]
    async fn test_too_many_input_objects_returns_400() {
        let (_test_cluster, _container, server) = start_rpc_server_for_testing(
            vec![MIST_PER_SUI; 10],
            MIST_PER_SUI,
            TEST_ADVANCED_FAUCET_MODE,
        )
        .await;

        let client = server.get_local_client();

        // Reserve gas coins first
        let (sponsor, reservation_id, gas_coins) =
            client.reserve_gas(MIST_PER_SUI, 10).await.unwrap();
        assert_eq!(gas_coins.len(), 1);

        // Create a transaction with more than 50 input objects (let's use 60)
        let sender = SuiAddress::random_for_testing_only();
        let (tx_data, user_sig) = create_transaction_with_many_input_objects(
            sender, sponsor, gas_coins, 60, // More than the MAX_INPUT_OBJECTS limit of 50
        );

        // Execute the transaction and expect it to fail with 400 Bad Request
        let result = client
            .execute_tx(reservation_id, &tx_data, &user_sig, None)
            .await;

        assert!(result.as_ref().unwrap().error.is_some());
        let error_msg = result.unwrap().error.unwrap();

        // Check that the error message contains the expected text about too many input objects
        assert!(error_msg.contains("Transaction has too many input objects"));
        assert!(error_msg.contains(&format!("{MAX_INPUT_OBJECTS}"))); // The maximum allowed
    }
}
