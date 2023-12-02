// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod client;
mod rpc_types;
mod server;

pub use server::GasStationServer;

#[cfg(test)]
mod tests {
    use crate::config::DEFAULT_RPC_PORT;
    use crate::rpc::client::GasStationRpcClient;
    use crate::rpc::rpc_types::{ReleaseGasRequest, ReserveGasRequest};
    use crate::rpc::GasStationServer;
    use crate::test_env::start_gas_station;
    use sui_types::gas_coin::MIST_PER_SUI;
    use sui_types::transaction::ProgrammableTransaction;
    use sui_types::transaction::TransactionDataAPI;

    #[tokio::test]
    async fn test_basic_rpc_flow() {
        telemetry_subscribers::init_for_testing();
        let (_test_cluster, station) = start_gas_station(vec![MIST_PER_SUI; 10]).await;
        let rpc_port = DEFAULT_RPC_PORT;
        let _server = GasStationServer::new(station, rpc_port).await;
        let client = GasStationRpcClient::new(format!("http://localhost:{}", rpc_port));
        client.check_health().await.unwrap();

        let mut request = ReserveGasRequest {
            programmable: ProgrammableTransaction {
                inputs: vec![],
                commands: vec![],
            },
            sender: Default::default(),
            expiration: None,
            budget: MIST_PER_SUI,
            price: 1000,
            reserve_duration_secs: 10,
        };
        let response = client.reserve_gas(request.clone()).await.unwrap();
        assert!(response.error.is_none());
        let tx = response.tx_and_sig.unwrap().0;
        let payment = &tx.value.gas_data().payment;
        assert_eq!(payment.len(), 1);
        request.budget = MIST_PER_SUI * 10;

        // We can no longer request all balance given one is loaned out above.
        assert!(client.reserve_gas(request.clone()).await.is_err());

        client
            .release_gas(ReleaseGasRequest {
                gas_coins: payment.iter().map(|oref| oref.0).collect(),
            })
            .await
            .unwrap();
        let response = client.reserve_gas(request).await.unwrap();
        assert!(response.error.is_none());
        assert_eq!(
            response
                .tx_and_sig
                .unwrap()
                .0
                .value
                .gas_data()
                .payment
                .len(),
            10
        );
    }
}
