// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;
use sui_types::base_types::SuiAddress;
use sui_types::signature::GenericSignature;
use sui_types::transaction::{TransactionData, TransactionDataAPI};

pub mod in_memory_signer;
pub mod sidecar_signer;

#[async_trait::async_trait]
pub trait TxSignerTrait: Send + Sync {
    async fn sign_transaction(&self, tx_data: &TransactionData)
        -> anyhow::Result<GenericSignature>;
    fn sui_address(&self) -> SuiAddress;
}

pub struct TxSigner {
    signers: Vec<Arc<dyn TxSignerTrait>>,
    next_signer_idx: AtomicUsize,
    address_index_map: HashMap<SuiAddress, usize>,
}

impl TxSigner {
    pub fn new(signers: Vec<Arc<dyn TxSignerTrait>>) -> Arc<Self> {
        let address_index_map: HashMap<_, _> = signers
            .iter()
            .enumerate()
            .map(|(i, s)| (s.sui_address(), i))
            .collect();
        Arc::new(Self {
            signers,
            next_signer_idx: AtomicUsize::new(0),
            address_index_map,
        })
    }

    pub fn get_all_addresses(&self) -> Vec<SuiAddress> {
        self.signers.iter().map(|s| s.sui_address()).collect()
    }

    pub fn is_valid_address(&self, address: &SuiAddress) -> bool {
        self.address_index_map.contains_key(address)
    }

    pub fn get_one_address(&self) -> SuiAddress {
        let idx = self.next_signer_idx.fetch_add(1, atomic::Ordering::Relaxed);
        self.signers[idx % self.signers.len()].sui_address()
    }

    pub async fn sign_transaction(
        &self,
        tx_data: &TransactionData,
    ) -> anyhow::Result<GenericSignature> {
        let sponsor_address = tx_data.gas_data().owner;
        let idx = *self
            .address_index_map
            .get(&sponsor_address)
            .ok_or_else(|| anyhow::anyhow!("No signer found for address: {}", sponsor_address))?;
        self.signers[idx].sign_transaction(tx_data).await
    }
}

#[cfg(test)]
mod tests {
    use in_memory_signer::InMemoryTxSigner;
    use sui_types::{
        crypto::get_account_key_pair,
        programmable_transaction_builder::ProgrammableTransactionBuilder,
        transaction::TransactionKind,
    };

    use super::*;

    #[tokio::test]
    async fn test_multi_tx_signer() {
        let (sender1, key1) = get_account_key_pair();
        let (sender2, key2) = get_account_key_pair();
        let (sender3, key3) = get_account_key_pair();
        let senders = vec![sender1, sender2, sender3];
        let signer1 = InMemoryTxSigner::new(key1.into());
        let signer2 = InMemoryTxSigner::new(key2.into());
        let signer3 = InMemoryTxSigner::new(key3.into());
        let tx_signer = TxSigner::new(vec![signer1, signer2, signer3]);
        for sender in senders {
            let tx_data = TransactionData::new_with_gas_coins(
                TransactionKind::ProgrammableTransaction(
                    ProgrammableTransactionBuilder::new().finish(),
                ),
                sender,
                vec![],
                0,
                0,
            );
            tx_signer.sign_transaction(&tx_data).await.unwrap();
        }
        let (sender4, _) = get_account_key_pair();
        let tx_data = TransactionData::new_with_gas_coins(
            TransactionKind::ProgrammableTransaction(
                ProgrammableTransactionBuilder::new().finish(),
            ),
            sender4,
            vec![],
            0,
            0,
        );
        assert!(tx_signer.sign_transaction(&tx_data).await.is_err());
    }
}
