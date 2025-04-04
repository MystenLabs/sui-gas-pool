// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use shared_crypto::intent::{Intent, IntentMessage};
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{Signature, SuiKeyPair};
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

use super::TxSignerTrait;

pub struct InMemoryTxSigner {
    keypair: SuiKeyPair,
}

impl InMemoryTxSigner {
    pub fn new(keypair: SuiKeyPair) -> Arc<Self> {
        Arc::new(Self { keypair })
    }
}

#[async_trait::async_trait]
impl TxSignerTrait for InMemoryTxSigner {
    async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data);
        let sponsor_sig = Signature::new_secure(&intent_msg, &self.keypair).into();
        Ok(sponsor_sig)
    }

    fn sui_address(&self) -> SuiAddress {
        (&self.keypair.public()).into()
    }
}
