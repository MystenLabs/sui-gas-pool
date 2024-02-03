// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::read_auth_env;
use crate::rpc::rpc_types::{
    ExecuteTxRequest, ExecuteTxResponse, ReserveGasRequest, ReserveGasResponse,
};
use crate::types::ReservationID;
use fastcrypto::encoding::Base64;
use reqwest::header::{HeaderMap, AUTHORIZATION};
use reqwest::Client;
use sui_json_rpc_types::SuiTransactionBlockEffects;
use sui_types::base_types::{ObjectRef, SuiAddress};
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

#[derive(Clone)]
pub struct GasPoolRpcClient {
    client: Client,
    server_address: String,
}

impl GasPoolRpcClient {
    pub fn new(server_address: String) -> Self {
        let client = Client::new();
        Self {
            client,
            server_address,
        }
    }

    pub async fn check_health(&self) -> Result<(), reqwest::Error> {
        self.client
            .get(format!("{}/", self.server_address))
            .send()
            .await?
            .text()
            .await?;
        Ok(())
    }

    pub async fn version(&self) -> Result<String, reqwest::Error> {
        self.client
            .get(format!("{}/version", self.server_address))
            .send()
            .await?
            .text()
            .await
    }

    pub async fn reserve_gas(
        &self,
        gas_budget: u64,
        request_sponsor: Option<SuiAddress>,
        reserve_duration_secs: u64,
    ) -> anyhow::Result<(SuiAddress, ReservationID, Vec<ObjectRef>)> {
        let request = ReserveGasRequest {
            gas_budget,
            request_sponsor,
            reserve_duration_secs,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", read_auth_env()).parse().unwrap(),
        );
        let response = self
            .client
            .post(format!("{}/v1/reserve_gas", self.server_address))
            .headers(headers)
            .json(&request)
            .send()
            .await?
            .json::<ReserveGasResponse>()
            .await?;
        response
            .result
            .ok_or_else(|| {
                anyhow::anyhow!(response
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string()))
            })
            .map(|result| {
                (
                    result.sponsor_address,
                    result.reservation_id,
                    result
                        .gas_coins
                        .into_iter()
                        .map(|c| c.to_object_ref())
                        .collect(),
                )
            })
    }

    pub async fn execute_tx(
        &self,
        reservation_id: ReservationID,
        tx_data: &TransactionData,
        user_sig: &GenericSignature,
    ) -> anyhow::Result<SuiTransactionBlockEffects> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", read_auth_env()).parse().unwrap(),
        );
        let request = ExecuteTxRequest {
            reservation_id,
            tx_bytes: Base64::from_bytes(&bcs::to_bytes(&tx_data).unwrap()),
            user_sig: Base64::from_bytes(user_sig.as_ref()),
        };
        let response = self
            .client
            .post(format!("{}/v1/execute_tx", self.server_address))
            .headers(headers)
            .json(&request)
            .send()
            .await?
            .json::<ExecuteTxResponse>()
            .await?;
        response.effects.ok_or_else(|| {
            anyhow::anyhow!(response
                .error
                .unwrap_or_else(|| "Unknown error".to_string()))
        })
    }
}
