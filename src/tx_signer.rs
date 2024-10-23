// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::anyhow;
use fastcrypto::encoding::{Base64, Encoding};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use shared_crypto::intent::{Intent, IntentMessage};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicUsize;
use std::sync::{atomic, Arc};
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{Signature, SuiKeyPair};
use sui_types::signature::GenericSignature;
use sui_types::transaction::{TransactionData, TransactionDataAPI};

#[async_trait::async_trait]
pub trait TxSigner: Send + Sync {
    async fn sign_transaction(&self, tx_data: &TransactionData)
        -> anyhow::Result<GenericSignature>;
    fn get_one_address(&self) -> SuiAddress;
    fn get_all_addresses(&self) -> Vec<SuiAddress>;
    fn is_valid_address(&self, address: &SuiAddress) -> bool;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignatureResponse {
    signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuiAddressResponse {
    sui_pubkey_address: SuiAddress,
}

// TODO: Add a mock side car server with tests for multi-address support.
pub struct SidecarTxSigner {
    client: Client,
    sidecar_url_map: HashMap<SuiAddress, String>,
    sui_addresses: Vec<SuiAddress>,
    next_address_idx: AtomicUsize,
}

impl SidecarTxSigner {
    pub async fn new(sidecar_urls: Vec<String>) -> Arc<Self> {
        let client = Client::new();
        let mut sidecar_url_map = HashMap::new();
        let mut sui_addresses = vec![];
        for sidecar_url in sidecar_urls {
            let resp = client
                .get(format!("{}/{}", &sidecar_url, "get-pubkey-address"))
                .send()
                .await
                .unwrap_or_else(|err| panic!("Failed to get pubkey address: {}", err));
            let sui_address = resp
                .json::<SuiAddressResponse>()
                .await
                .unwrap_or_else(|err| panic!("Failed to parse address response: {}", err))
                .sui_pubkey_address;
            sui_addresses.push(sui_address);
            sidecar_url_map.insert(sui_address, sidecar_url);
        }
        Arc::new(Self {
            client,
            sidecar_url_map,
            sui_addresses,
            next_address_idx: AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl TxSigner for SidecarTxSigner {
    async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let sponsor_address = tx_data.gas_data().owner;
        let sidecar_url = self
            .sidecar_url_map
            .get(&sponsor_address)
            .ok_or_else(|| anyhow!("Address is not a valid sponsor: {:?}", sponsor_address))?;
        let bytes = Base64::encode(bcs::to_bytes(&tx_data)?);
        let resp = self
            .client
            .post(format!("{}/{}", sidecar_url, "sign-transaction"))
            .header("Content-Type", "application/json")
            .json(&json!({"txBytes": bytes}))
            .send()
            .await?;
        let sig_bytes = resp.json::<SignatureResponse>().await?;
        let sig = GenericSignature::from_str(&sig_bytes.signature)
            .map_err(|err| anyhow!(err.to_string()))?;
        Ok(sig)
    }

    fn get_one_address(&self) -> SuiAddress {
        // Round robin the address we are using.
        let idx = self
            .next_address_idx
            .fetch_add(1, atomic::Ordering::Relaxed);
        self.sui_addresses[idx % self.sui_addresses.len()]
    }

    fn get_all_addresses(&self) -> Vec<SuiAddress> {
        self.sui_addresses.clone()
    }

    fn is_valid_address(&self, address: &SuiAddress) -> bool {
        self.sidecar_url_map.contains_key(address)
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

    fn get_one_address(&self) -> SuiAddress {
        (&self.keypair.public()).into()
    }

    fn get_all_addresses(&self) -> Vec<SuiAddress> {
        vec![self.get_one_address()]
    }

    fn is_valid_address(&self, address: &SuiAddress) -> bool {
        address == &self.get_one_address()
    }
}
