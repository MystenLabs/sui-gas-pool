// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::rpc::client::GasPoolRpcClient;
use clap::ValueEnum;
use parking_lot::RwLock;
use shared_crypto::intent::{Intent, IntentMessage};
use std::sync::Arc;
use sui_config::node::DEFAULT_VALIDATOR_GAS_PRICE;
use sui_types::base_types::SuiAddress;
use sui_types::crypto::{get_account_key_pair, Signature};
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::transaction::{Argument, TransactionData, TransactionKind};
use tokio::time::{interval, Duration, Instant};

#[derive(Copy, Clone, ValueEnum)]
pub enum BenchmarkMode {
    ReserveOnly,
    ReserveAndExecute,
}

#[derive(Clone, Default)]
struct BenchmarkStatsPerSecond {
    pub num_requests: u64,
    pub total_latency: u128,
    pub num_errors: u64,
}

impl BenchmarkMode {
    pub async fn run_benchmark(
        &self,
        gas_station_url: String,
        reserve_duration_sec: u64,
        num_clients: u64,
    ) {
        let mut handles = vec![];
        let stats = Arc::new(RwLock::new(BenchmarkStatsPerSecond::default()));
        let client = GasPoolRpcClient::new(gas_station_url);
        let should_execute = matches!(self, Self::ReserveAndExecute);
        for _ in 0..num_clients {
            let client = client.clone();
            let stats = stats.clone();
            let handle = tokio::spawn(async move {
                let (sender, keypair) = get_account_key_pair();
                loop {
                    let now = Instant::now();
                    let budget = 100000000;
                    let result = client.reserve_gas(budget, None, reserve_duration_sec).await;
                    println!("result: {:?}", result);
                    let Ok((sponsor, reservation_id, gas_coins)) = result else {
                        stats.write().num_errors += 1;
                        continue;
                    };
                    stats.write().num_requests += 1;
                    stats.write().total_latency += now.elapsed().as_millis();
                    if !should_execute {
                        continue;
                    }

                    let mut pt_builder = ProgrammableTransactionBuilder::new();
                    pt_builder.transfer_arg(SuiAddress::ZERO, Argument::GasCoin);
                    let pt = pt_builder.finish();
                    let tx_data = TransactionData::new_with_gas_coins_allow_sponsor(
                        TransactionKind::ProgrammableTransaction(pt),
                        sender,
                        gas_coins,
                        budget,
                        DEFAULT_VALIDATOR_GAS_PRICE,
                        sponsor,
                    );
                    let intent_msg = IntentMessage::new(Intent::sui_transaction(), &tx_data);
                    let user_sig = Signature::new_secure(&intent_msg, &keypair).into();
                    println!(
                        "result: {:?}",
                        client.execute_tx(reservation_id, &tx_data, &user_sig).await
                    );
                }
            });
            handles.push(handle);
        }
        let handle = tokio::spawn(async move {
            let mut prev_stats = stats.read().clone();
            let mut interval = interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let cur_stats = stats.read().clone();
                let request_per_second = cur_stats.num_requests - prev_stats.num_requests;
                println!(
                    "Requests per second: {}, errors per second: {}, average latency: {}ms",
                    request_per_second,
                    cur_stats.num_errors - prev_stats.num_errors,
                    if request_per_second == 0 {
                        0
                    } else {
                        (cur_stats.total_latency - prev_stats.total_latency)
                            / request_per_second as u128
                    }
                );
                prev_stats = cur_stats;
            }
        });
        handle.await.unwrap();
    }
}
