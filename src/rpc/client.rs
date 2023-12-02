// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::rpc::rpc_types::{ReleaseGasRequest, ReserveGasRequest, ReserveGasResponse};
use reqwest::Client;

pub struct GasStationRpcClient {
    client: Client,
    server_address: String,
}

impl GasStationRpcClient {
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

    pub async fn reserve_gas(
        &self,
        request: ReserveGasRequest,
    ) -> Result<ReserveGasResponse, reqwest::Error> {
        self.client
            .post(format!("{}/v1/reserve_gas", self.server_address))
            .json(&request)
            .send()
            .await?
            .json::<ReserveGasResponse>()
            .await
    }

    pub async fn release_gas(&self, request: ReleaseGasRequest) -> Result<(), reqwest::Error> {
        self.client
            .post(format!("{}/v1/release_gas", self.server_address))
            .json(&request)
            .send()
            .await?
            .json::<()>()
            .await
    }
}
