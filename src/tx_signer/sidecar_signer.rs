// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::anyhow;
use fastcrypto::encoding::{Base64, Encoding};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use sui_types::base_types::SuiAddress;
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;

use super::TxSignerTrait;

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

pub struct SidecarTxSigner {
    client: Client,
    sidecar_url: String,
    sui_address: SuiAddress,
}

impl SidecarTxSigner {
    pub async fn new(sidecar_url: String) -> Arc<Self> {
        let client = Client::new();
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
        Arc::new(Self {
            client,
            sidecar_url,
            sui_address,
        })
    }
}

#[async_trait::async_trait]
impl TxSignerTrait for SidecarTxSigner {
    async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let bytes = Base64::encode(bcs::to_bytes(&tx_data)?);
        let resp = self
            .client
            .post(format!("{}/{}", self.sidecar_url, "sign-transaction"))
            .header("Content-Type", "application/json")
            .json(&json!({"txBytes": bytes}))
            .send()
            .await?;
        let sig_bytes = resp.json::<SignatureResponse>().await?;
        let sig = GenericSignature::from_str(&sig_bytes.signature)
            .map_err(|err| anyhow!(err.to_string()))?;
        Ok(sig)
    }

    fn sui_address(&self) -> SuiAddress {
        self.sui_address
    }
}
