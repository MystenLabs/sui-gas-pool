// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use shared_crypto::intent::{Intent, IntentMessage};
use std::sync::Arc;
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{Signature, SuiKeyPair};
use sui_types::transaction::TransactionData;

#[async_trait::async_trait]
pub trait TxSigner: Send + Sync {
    async fn sign_transaction(&self, tx_data: &TransactionData) -> anyhow::Result<Signature>;
    fn get_address(&self) -> SuiAddress;
    fn is_valid_address(&self, address: &SuiAddress) -> bool;
}

pub struct TestTxSigner {
    keypair: SuiKeyPair,
}

impl TestTxSigner {
    pub fn new(keypair: SuiKeyPair) -> Arc<Self> {
        Arc::new(Self { keypair })
    }
}

#[async_trait::async_trait]
impl TxSigner for TestTxSigner {
    async fn sign_transaction(&self, tx_data: &TransactionData) -> anyhow::Result<Signature> {
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data);
        let sponsor_sig = Signature::new_secure(&intent_msg, &self.keypair);
        Ok(sponsor_sig)
    }

    fn get_address(&self) -> SuiAddress {
        (&self.keypair.public()).into()
    }

    fn is_valid_address(&self, address: &SuiAddress) -> bool {
        self.get_address() == *address
    }
}
