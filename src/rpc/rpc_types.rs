// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use sui_json_rpc_types::SuiTransactionBlockEffects;
use sui_types::base_types::{ObjectRef, SuiAddress};
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReserveGasRequest {
    pub gas_budget: u64,
    pub request_sponsor: Option<SuiAddress>,
    pub reserve_duration_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReserveGasResponse {
    pub gas_coins: Option<(SuiAddress, Vec<ObjectRef>)>,
    pub error: Option<String>,
}

impl ReserveGasResponse {
    pub fn new_ok(sponsor: SuiAddress, gas_coins: Vec<ObjectRef>) -> Self {
        Self {
            gas_coins: Some((sponsor, gas_coins)),
            error: None,
        }
    }

    pub fn new_err(error: anyhow::Error) -> Self {
        Self {
            gas_coins: None,
            error: Some(error.to_string()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteTxRequest {
    pub tx: TransactionData,
    pub user_sig: GenericSignature,
}

#[derive(Debug, Serialize, Deserialize)]
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
