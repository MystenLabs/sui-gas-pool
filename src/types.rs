// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use sui_types::base_types::ObjectRef;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GasCoin {
    pub object_ref: ObjectRef,
    pub balance: u64,
}
