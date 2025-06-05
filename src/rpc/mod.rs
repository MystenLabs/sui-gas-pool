// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod client;
mod rpc_types;
mod server;

pub use server::GasPoolServer;

#[cfg(test)]
mod tests {
    use crate::test_env::{create_test_transaction, start_rpc_server_for_testing};
    use crate::AUTH_ENV_NAME;
    use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
    use sui_types::gas_coin::MIST_PER_SUI;

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
        let effects = client
            .execute_tx(reservation_id, &tx_data, &user_sig)
            .await
            .unwrap();
        assert!(effects.status().is_ok());
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
        std::env::set_var(AUTH_ENV_NAME, "b");
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
}
