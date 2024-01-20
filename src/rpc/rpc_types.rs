// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::types::ReservationID;
use fastcrypto::encoding::Base64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sui_json_rpc_types::{SuiObjectRef, SuiTransactionBlockEffects};
use sui_types::base_types::{ObjectRef, SuiAddress};

#[derive(Clone, Debug, JsonSchema, Serialize, Deserialize)]
pub struct ReserveGasRequest {
    pub gas_budget: u64,
    pub request_sponsor: Option<SuiAddress>,
    pub reserve_duration_secs: u64,
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
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ExecuteTxResponse {
    pub effects: Option<SuiTransactionBlockEffects>,
    pub error: Option<String>,
}

impl ExecuteTxResponse {
    pub fn new_ok(effects: SuiTransactionBlockEffects) -> Self {
        Self {
            effects: Some(effects),
            error: None,
        }
    }

    pub fn new_err(error: anyhow::Error) -> Self {
        Self {
            effects: None,
            error: Some(error.to_string()),
        }
    }
}
