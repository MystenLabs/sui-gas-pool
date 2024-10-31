// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::tx_signer::{SidecarTxSigner, TxSigner};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sui_types::base_types::{random_object_ref, SuiAddress};
use sui_types::transaction::{ProgrammableTransaction, TransactionData, TransactionKind};

pub async fn run_kms_stress_test(kms_url: String, num_tasks: usize) {
    let signer = SidecarTxSigner::new(kms_url).await;
    let test_tx_data = TransactionData::new(
        TransactionKind::ProgrammableTransaction(ProgrammableTransaction {
            inputs: vec![],
            commands: vec![],
        }),
        SuiAddress::ZERO,
        random_object_ref(),
        1000,
        1000,
    );
    // Shared atomic counters for successes and failures
    let success_counter = Arc::new(AtomicUsize::new(0));
    let failure_counter = Arc::new(AtomicUsize::new(0));

    // Shared atomic flag to signal workers to stop
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Vector to hold worker task handles
    let mut handles = Vec::with_capacity(num_tasks);

    for _ in 0..num_tasks {
        let success_counter = Arc::clone(&success_counter);
        let failure_counter = Arc::clone(&failure_counter);
        let stop_flag = Arc::clone(&stop_flag);
        let signer = signer.clone(); // Ensure Signer is Clone
        let test_tx_data = test_tx_data.clone(); // Clone the test data

        // Spawn a worker task
        let handle = tokio::spawn(async move {
            while !stop_flag.load(Ordering::Relaxed) {
                match signer.sign_transaction(&test_tx_data).await {
                    Ok(_) => {
                        // Increment the success counter on successful call
                        success_counter.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        // Increment the failure counter on failed call
                        failure_counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        handles.push(handle);
    }

    // Start the timer
    let start = Instant::now();

    // Let the test run for the specified duration
    tokio::time::sleep(Duration::from_secs(30)).await;

    // Signal all workers to stop
    stop_flag.store(true, Ordering::Relaxed);

    // Wait for all workers to finish
    for handle in handles {
        let _ = handle.await;
    }

    // Calculate elapsed time
    let elapsed = start.elapsed().as_secs_f64();

    // Get the total number of successful and failed calls
    let total_success = success_counter.load(Ordering::Relaxed);
    let total_failures = failure_counter.load(Ordering::Relaxed);
    let total_calls = total_success + total_failures;

    // Calculate throughput
    let throughput = total_success as f64 / elapsed;

    println!("Total calls completed: {}", total_calls);
    println!(" - Successful: {}", total_success);
    println!(" - Failed: {}", total_failures);
    println!("Elapsed time: {:.2} seconds", elapsed);
    println!("Throughput: {:.2} successful calls/second", throughput);
}
