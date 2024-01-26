// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::rpc::client::GasPoolRpcClient;
use clap::ValueEnum;
use parking_lot::RwLock;
use rand::rngs::OsRng;
use rand::Rng;
use shared_crypto::intent::{Intent, IntentMessage};
use std::sync::Arc;
use sui_config::node::DEFAULT_VALIDATOR_GAS_PRICE;
use sui_types::crypto::{get_account_key_pair, Signature};
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
use sui_types::transaction::{TransactionData, TransactionKind};
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

impl BenchmarkStatsPerSecond {
    pub fn update_success(&mut self, latency: u128) {
        self.num_requests += 1;
        self.total_latency += latency;
    }

    pub fn update_error(&mut self) {
        self.num_requests += 1;
        self.num_errors += 1;
    }
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
                let mut rng = OsRng;
                loop {
                    let now = Instant::now();
                    let budget = rng.gen_range(1_000_000u64..100_000_000u64);
                    let result = client.reserve_gas(budget, None, reserve_duration_sec).await;
                    let (sponsor, reservation_id, gas_coins) = match result {
                        Ok(r) => r,
                        Err(err) => {
                            stats.write().update_error();
                            println!("Error: {}", err);
                            continue;
                        }
                    };
                    if !should_execute {
                        stats.write().update_success(now.elapsed().as_millis());
                        continue;
                    }

                    let pt_builder = ProgrammableTransactionBuilder::new();
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
                    let result = client.execute_tx(reservation_id, &tx_data, &user_sig).await;
                    if let Err(err) = result {
                        stats.write().update_error();
                        println!("Error: {}", err);
                    } else {
                        stats.write().update_success(now.elapsed().as_millis());
                    }
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
                let num_errors = cur_stats.num_errors - prev_stats.num_errors;
                println!(
                    "Requests per second: {}, errors per second: {}, average latency: {}ms",
                    request_per_second,
                    num_errors,
                    if request_per_second == 0 {
                        0
                    } else {
                        (cur_stats.total_latency - prev_stats.total_latency)
                            / ((request_per_second - num_errors) as u128)
                    }
                );
                prev_stats = cur_stats;
            }
        });
        handle.await.unwrap();
    }
}
