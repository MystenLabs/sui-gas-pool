// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::anyhow;
use fastcrypto::encoding::{Base64, Encoding};
use reqwest::Client;
use serde_json::json;
use shared_crypto::intent::{Intent, IntentMessage};
use std::str::FromStr;
use std::sync::Arc;
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{Signature, SuiKeyPair};
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

#[async_trait::async_trait]
pub trait TxSigner: Send + Sync {
    async fn sign_transaction(&self, tx_data: &TransactionData)
        -> anyhow::Result<GenericSignature>;
    fn get_address(&self) -> SuiAddress;
    fn is_valid_address(&self, address: &SuiAddress) -> bool {
        self.get_address() == *address
    }
}

pub struct SidecarTxSigner {
    sponsor_address: SuiAddress,
    sidecar_url: String,
    client: Client,
}

impl SidecarTxSigner {
    pub fn new(sponsor_address: SuiAddress, sidecar_url: String) -> Arc<Self> {
        Arc::new(Self {
            sponsor_address,
            sidecar_url,
            client: Client::new(),
        })
    }
}

#[async_trait::async_trait]
impl TxSigner for SidecarTxSigner {
    async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let bytes = Base64::encode(bcs::to_bytes(&tx_data)?);
        let resp = self
            .client
            .post(self.sidecar_url.clone())
            .header("Content-Type", "application/json")
            .json(&json!({"txBytes": bytes}))
            .send()
            .await?;
        tracing::debug!("sign_transaction response: {:?}", resp);
        let sig_bytes = resp.json::<String>().await?;
        let sig = GenericSignature::from_str(&sig_bytes).map_err(|err| anyhow!(err.to_string()))?;
        Ok(sig)
    }

    fn get_address(&self) -> SuiAddress {
        self.sponsor_address
    }
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
    async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data);
        let sponsor_sig = Signature::new_secure(&intent_msg, &self.keypair).into();
        Ok(sponsor_sig)
    }

    fn get_address(&self) -> SuiAddress {
        (&self.keypair.public()).into()
    }
}
