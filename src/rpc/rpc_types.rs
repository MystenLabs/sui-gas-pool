// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use shared_crypto::intent::IntentMessage;
use sui_types::base_types::{EpochId, ObjectID, SuiAddress};
use sui_types::signature::GenericSignature;
use sui_types::transaction::{ProgrammableTransaction, TransactionData};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReserveGasRequest {
    pub programmable: ProgrammableTransaction,
    pub sender: SuiAddress,
    pub expiration: Option<EpochId>,
    pub budget: u64,
    pub price: u64,
    pub reserve_duration_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReserveGasResponse {
    pub tx_and_sig: Option<(IntentMessage<TransactionData>, GenericSignature)>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReleaseGasRequest {
    pub gas_coins: Vec<ObjectID>,
}
