// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::rpc::GasStationServer;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use sui_types::gas_coin::MIST_PER_SUI;
use tokio::time::{interval, Duration};
use tracing::info;

pub async fn benchmark_reserve_only() {
    info!("Setting up gas station...");
    let (_test_cluster, _container, server) =
        GasStationServer::start_rpc_server_for_testing(vec![1000000 * MIST_PER_SUI; 1]).await;
    info!("Gas station is ready");
    let mut handles = vec![];
    let num_requesets = Arc::new(AtomicU64::new(0));
    for _ in 0..1 {
        let client = server.get_local_client();
        let num_requesets = num_requesets.clone();
        let handle = tokio::spawn(async move {
            loop {
                let result = client.reserve_gas(1, None, 1).await;
                if result.is_ok() {
                    num_requesets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        handles.push(handle);
    }
    let handle = tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(1));
        let mut prev_num_requests = num_requesets.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            interval.tick().await;
            let current_num_requests = num_requesets.load(std::sync::atomic::Ordering::SeqCst);
            println!(
                "Requests per second: {}",
                current_num_requests - prev_num_requests
            );
            prev_num_requests = current_num_requests;
        }
    });
    handle.await.unwrap();
}
