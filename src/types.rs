// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use shared_crypto::intent::IntentMessage;
use sui_types::base_types::{ObjectRef, SuiAddress};
use sui_types::signature::AuthenticatorTrait;
use sui_types::signature::{GenericSignature, VerifyParams};
use sui_types::transaction::TransactionDataAPI;
use sui_types::transaction::{
    ProgrammableTransaction, TransactionData, TransactionExpiration, TransactionKind,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GasCoin {
    pub object_ref: ObjectRef,
    pub balance: u64,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct GaslessTransaction {
    pub pt: ProgrammableTransaction,
    pub sender: SuiAddress,
    pub expiration: TransactionExpiration,
    pub budget: u64,
    pub price: u64,
}

impl Default for GaslessTransaction {
    fn default() -> Self {
        Self {
            pt: ProgrammableTransaction {
                inputs: vec![],
                commands: vec![],
            },
            sender: SuiAddress::default(),
            expiration: TransactionExpiration::None,
            budget: 1000,
            price: 1000,
        }
    }
}

impl GaslessTransaction {
    pub fn check_consistent_with(
        &self,
        transaction: &IntentMessage<TransactionData>,
        signature: &GenericSignature,
    ) -> anyhow::Result<()> {
        signature.verify_claims(
            transaction,
            transaction.value.gas_data().owner,
            &VerifyParams::default(),
            signature.check_author(),
        )?;
        if self.sender != transaction.value.sender() {
            anyhow::bail!("Sender does not match");
        }
        if self.budget != transaction.value.gas_data().budget {
            anyhow::bail!("Budget does not match");
        }
        if self.price != transaction.value.gas_data().price {
            anyhow::bail!("Price does not match");
        }
        if &self.expiration != transaction.value.expiration() {
            anyhow::bail!("Expiration does not match");
        }
        if let TransactionKind::ProgrammableTransaction(pt) = transaction.value.kind() {
            if &self.pt != pt {
                anyhow::bail!("ProgrammableTransaction does not match");
            }
        } else {
            anyhow::bail!("Transaction kind does not match");
        }

        Ok(())
    }
}
