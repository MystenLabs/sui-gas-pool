// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use sui_json_rpc_types::SuiObjectRef;
use sui_types::base_types::{ObjectID, ObjectRef};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GasCoin {
    pub object_ref: ObjectRef,
    pub balance: u64,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct SuiGasCoin {
    pub object_ref: SuiObjectRef,
    pub balance: u64,
}

impl From<GasCoin> for SuiGasCoin {
    fn from(gas_coin: GasCoin) -> Self {
        Self {
            object_ref: gas_coin.object_ref.into(),
            balance: gas_coin.balance,
        }
    }
}

impl From<SuiGasCoin> for GasCoin {
    fn from(gas_coin: SuiGasCoin) -> Self {
        Self {
            object_ref: gas_coin.object_ref.to_object_ref(),
            balance: gas_coin.balance,
        }
    }
}

pub type ExpirationTimeMs = u64;
pub type GasGroupKey = ObjectID;

#[derive(Clone, Default, Debug)]
pub struct UpdatedGasGroup {
    pub updated_gas_coins: Vec<GasCoin>,
    pub deleted_gas_coins: Vec<ObjectID>,
}

impl UpdatedGasGroup {
    pub fn new(updated_gas_coins: Vec<GasCoin>, deleted_gas_coins: Vec<ObjectID>) -> Self {
        Self {
            updated_gas_coins,
            deleted_gas_coins,
        }
    }
    pub fn get_group_key(&self) -> GasGroupKey {
        let all_ids: BTreeSet<_> = self
            .updated_gas_coins
            .iter()
            .map(|coin| &coin.object_ref.0)
            .chain(&self.deleted_gas_coins)
            .collect();
        assert!(all_ids.len() > 0);
        assert_eq!(
            all_ids.len(),
            self.updated_gas_coins.len() + self.deleted_gas_coins.len()
        );
        *all_ids.into_iter().next().unwrap()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReservedGasGroup {
    pub objects: BTreeSet<ObjectID>,
    pub expiration_time: ExpirationTimeMs,
}

impl ReservedGasGroup {
    pub fn get_key(&self) -> GasGroupKey {
        *self.objects.iter().next().unwrap()
    }
}
