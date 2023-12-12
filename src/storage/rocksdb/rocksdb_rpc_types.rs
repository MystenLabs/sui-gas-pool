// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::types::GasCoin;
use serde::{Deserialize, Serialize};
use sui_types::base_types::{ObjectID, SuiAddress};

#[derive(Debug, Serialize, Deserialize)]
pub struct ReserveGasStorageRequest {
    pub gas_budget: u64,
    pub request_sponsor: SuiAddress,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateGasStorageRequest {
    pub sponsor_address: SuiAddress,
    pub released_gas_coins: Vec<GasCoin>,
    pub deleted_gas_coins: Vec<ObjectID>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReserveGasStorageResponse {
    pub gas_coins: Option<Vec<GasCoin>>,
    pub error: Option<String>,
}

impl ReserveGasStorageResponse {
    pub fn new_ok(gas_coins: Vec<GasCoin>) -> Self {
        Self {
            gas_coins: Some(gas_coins),
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
pub struct UpdateGasStorageResponse {
    pub error: Option<String>,
}

impl UpdateGasStorageResponse {
    pub fn new_ok() -> Self {
        Self { error: None }
    }

    pub fn new_err(error: anyhow::Error) -> Self {
        Self {
            error: Some(error.to_string()),
        }
    }
}
