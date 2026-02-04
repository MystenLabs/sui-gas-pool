// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::types::ReservationID;
use fastcrypto::encoding::Base64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sui_json_rpc_types::{
    SuiObjectRef, SuiTransactionBlockEffects, SuiTransactionBlockResponse,
    SuiTransactionBlockResponseOptions,
};
use sui_types::base_types::{ObjectRef, SuiAddress};

// 10 mins.
pub const MAX_DURATION_S: u64 = 10 * 60;

#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ReserveGasRequest {
    pub gas_budget: u64,
    pub reserve_duration_secs: u64,
}

impl ReserveGasRequest {
    pub fn check_validity(&self, max_sui_per_request: u64) -> anyhow::Result<()> {
        if self.gas_budget == 0 {
            anyhow::bail!("Gas budget must be positive");
        }
        if self.gas_budget > max_sui_per_request {
            anyhow::bail!("Gas budget must be less than {}", max_sui_per_request);
        }
        if self.reserve_duration_secs == 0 {
            anyhow::bail!("Reserve duration must be positive");
        }
        if self.reserve_duration_secs > MAX_DURATION_S {
            anyhow::bail!(
                "Reserve duration must be less than {} seconds",
                MAX_DURATION_S
            );
        }
        Ok(())
    }
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ReserveGasResponse {
    pub result: Option<ReserveGasResult>,
    pub error: Option<String>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ReserveGasResult {
    pub sponsor_address: SuiAddress,
    pub reservation_id: ReservationID,
    pub gas_coins: Vec<SuiObjectRef>,
}

impl ReserveGasResponse {
    pub fn new_ok(
        sponsor_address: SuiAddress,
        reservation_id: ReservationID,
        gas_coins: Vec<ObjectRef>,
    ) -> Self {
        Self {
            result: Some(ReserveGasResult {
                sponsor_address,
                reservation_id,
                gas_coins: gas_coins.into_iter().map(|c| c.into()).collect(),
            }),
            error: None,
        }
    }

    pub fn new_err(error: anyhow::Error) -> Self {
        Self {
            result: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ExecuteTxRequest {
    pub reservation_id: ReservationID,
    pub tx_bytes: Base64,
    pub user_sig: Base64,
    pub options: Option<SuiTransactionBlockResponseOptions>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ExecuteTxResponse {
    pub effects: Option<SuiTransactionBlockEffects>,
    pub tx_block_response: Option<SuiTransactionBlockResponse>,
    pub error: Option<String>,
}

impl ExecuteTxResponse {
    pub fn new_ok_effects(effects: SuiTransactionBlockEffects) -> Self {
        Self {
            effects: Some(effects),
            tx_block_response: None,
            error: None,
        }
    }

    pub fn new_ok_block_response(tx_block_response: SuiTransactionBlockResponse) -> Self {
        Self {
            effects: None,
            tx_block_response: Some(tx_block_response),
            error: None,
        }
    }

    pub fn new_err(error: anyhow::Error) -> Self {
        Self {
            effects: None,
            tx_block_response: None,
            error: Some(error.to_string()),
        }
    }
}
