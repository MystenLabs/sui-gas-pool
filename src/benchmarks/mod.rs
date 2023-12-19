// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::rpc::client::GasStationRpcClient;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::time::{interval, Duration};

pub async fn run_benchmark(gas_station_url: String, reserve_duration_sec: u64) {
    let mut handles = vec![];
    let num_requests = Arc::new(AtomicU64::new(0));
    for _ in 0..10 {
        let client = GasStationRpcClient::new(gas_station_url.clone());
        let num_requesets = num_requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let result = client.reserve_gas(1, None, reserve_duration_sec).await;
                if result.is_ok() {
                    num_requesets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        handles.push(handle);
    }
    let handle = tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(1));
        let mut prev_num_requests = num_requests.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            interval.tick().await;
            let current_num_requests = num_requests.load(std::sync::atomic::Ordering::SeqCst);
            println!(
                "Requests per second: {}",
                current_num_requests - prev_num_requests
            );
            prev_num_requests = current_num_requests;
        }
    });
    handle.await.unwrap();
}
