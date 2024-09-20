// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use mysten_metrics::histogram::Histogram;
use prometheus::{
    register_int_counter_vec_with_registry, register_int_counter_with_registry,
    register_int_gauge_vec_with_registry, IntCounter, IntCounterVec, IntGaugeVec, Registry,
};
use std::sync::Arc;
use tracing::error;

pub struct GasPoolRpcMetrics {
    // === RPC Server Metrics ===
    // RPC metrics for the reserve_gas endpoint
    pub num_reserve_gas_requests: IntCounter,
    pub num_authorized_reserve_gas_requests: IntCounter,
    pub num_successful_reserve_gas_requests: IntCounter,
    pub num_failed_reserve_gas_requests: IntCounter,

    // Statistics about the gas reservation request
    pub target_gas_budget_per_request: Histogram,
    pub reserve_duration_per_request: Histogram,

    // RPC metrics for the execute_tx endpoint
    pub num_execute_tx_requests: IntCounter,
    pub num_authorized_execute_tx_requests: IntCounter,
    pub num_successful_execute_tx_requests: IntCounter,
    pub num_failed_execute_tx_requests: IntCounter,
}

impl GasPoolRpcMetrics {
    pub fn new(registry: &Registry) -> Arc<Self> {
        Arc::new(Self {
            num_reserve_gas_requests: register_int_counter_with_registry!(
                "num_reserve_gas_requests",
                "Total number of reserve_gas RPC requests received",
                registry,
            )
            .unwrap(),
            num_authorized_reserve_gas_requests: register_int_counter_with_registry!(
                "num_authorized_reserve_gas_requests",
                "Total number of reserve_gas RPC requests that provided the correct auth token",
                registry,
            )
            .unwrap(),
            num_successful_reserve_gas_requests: register_int_counter_with_registry!(
                "num_successful_reserve_gas_requests",
                "Total number of reserve_gas RPC requests that were successful",
                registry,
            )
            .unwrap(),
            num_failed_reserve_gas_requests: register_int_counter_with_registry!(
                "num_failed_reserve_gas_requests",
                "Total number of reserve_gas RPC requests that failed",
                registry,
            )
            .unwrap(),
            target_gas_budget_per_request: Histogram::new_in_registry(
                "target_gas_budget_per_request",
                "Target gas budget value in the reserve_gas RPC request",
                registry,
            ),
            reserve_duration_per_request: Histogram::new_in_registry(
                "reserve_duration_per_request",
                "Reserve duration value in the reserve_gas RPC request",
                registry,
            ),
            num_execute_tx_requests: register_int_counter_with_registry!(
                "num_execute_tx_requests",
                "Total number of execute_tx RPC requests received",
                registry,
            )
            .unwrap(),
            num_authorized_execute_tx_requests: register_int_counter_with_registry!(
                "num_authorized_execute_tx_requests",
                "Total number of execute_tx RPC requests that provided the correct auth token",
                registry,
            )
            .unwrap(),
            num_successful_execute_tx_requests: register_int_counter_with_registry!(
                "num_successful_execute_tx_requests",
                "Total number of execute_tx RPC requests that were successful",
                registry,
            )
            .unwrap(),
            num_failed_execute_tx_requests: register_int_counter_with_registry!(
                "num_failed_execute_tx_requests",
                "Total number of execute_tx RPC requests that failed",
                registry,
            )
            .unwrap(),
        })
    }

    pub fn new_for_testing() -> Arc<Self> {
        Self::new(&Registry::new())
    }
}

pub struct GasPoolCoreMetrics {
    pub num_expired_gas_coins: IntCounterVec,
    pub num_smashed_gas_coins: IntCounterVec,
    pub reserved_gas_coin_count_per_request: Histogram,
    pub reserve_gas_latency_ms: Histogram,
    pub transaction_signing_latency_ms: Histogram,
    pub transaction_execution_latency_ms: Histogram,
    pub num_gas_pool_invariant_violations: IntCounter,
    pub daily_gas_usage: IntGaugeVec,
}

impl GasPoolCoreMetrics {
    pub fn new(registry: &Registry) -> Arc<Self> {
        Arc::new(Self {
            reserved_gas_coin_count_per_request: Histogram::new_in_registry(
                "reserved_gas_coin_count_per_request",
                "Number of gas coins reserved in each reserve_gas RPC request",
                registry,
            ),
            num_expired_gas_coins: register_int_counter_vec_with_registry!(
                "num_expired_gas_coins",
                "Total number of gas coins that are put back due to reservation expiration",
                &["sponsor"],
                registry,
            )
                .unwrap(),
            num_smashed_gas_coins: register_int_counter_vec_with_registry!(
                "num_smashed_gas_coins",
                "Total number of gas coins that are smashed (i.e. deleted) during transaction execution",
                &["sponsor"],
                registry,
            )
                .unwrap(),
            reserve_gas_latency_ms: Histogram::new_in_registry(
                "reserve_gas_latency",
                "Latency of gas reservation, in milliseconds",
                registry,
            ),
            transaction_signing_latency_ms: Histogram::new_in_registry(
                "transaction_signing_latency",
                "Latency of transaction signing, in milliseconds",
                registry,
            ),
            transaction_execution_latency_ms: Histogram::new_in_registry(
                "transaction_execution_latency",
                "Latency of transaction execution, in milliseconds",
                registry,
            ),
            num_gas_pool_invariant_violations: register_int_counter_with_registry!(
                "num_gas_pool_invariant_violations",
                "Total number of invariant violations in the gas pool core",
                registry,
            )
                .unwrap(),
            daily_gas_usage: register_int_gauge_vec_with_registry!(
                "daily_gas_usage",
                "Current daily gas usage",
                &["sponsor"],
                registry,
            )
                .unwrap(),
        })
    }

    pub fn new_for_testing() -> Arc<Self> {
        Self::new(&Registry::new())
    }

    pub fn invariant_violation<T: Into<String>>(&self, msg: T) {
        if cfg!(debug_assertions) {
            panic!("Invariant violation: {}", msg.into());
        } else {
            error!("Invariant violation: {}", msg.into());
        }
        self.num_gas_pool_invariant_violations.inc();
    }
}

pub struct StorageMetrics {
    pub gas_pool_available_gas_coin_count: IntGaugeVec,
    pub gas_pool_available_gas_total_balance: IntGaugeVec,

    pub num_reserve_gas_coins_requests: IntCounter,
    pub num_successful_reserve_gas_coins_requests: IntCounter,
    pub num_ready_for_execution_requests: IntCounter,
    pub num_successful_ready_for_execution_requests: IntCounter,
    pub num_add_new_coins_requests: IntCounter,
    pub num_successful_add_new_coins_requests: IntCounter,
    pub num_expire_coins_requests: IntCounter,
    pub num_successful_expire_coins_requests: IntCounter,
}

impl StorageMetrics {
    pub fn new(registry: &Registry) -> Arc<Self> {
        Arc::new(Self {
            gas_pool_available_gas_coin_count: register_int_gauge_vec_with_registry!(
                "gas_pool_available_gas_coin_count",
                "Current number of available gas coins for reservation",
                &["sponsor"],
                registry,
            )
            .unwrap(),
            gas_pool_available_gas_total_balance: register_int_gauge_vec_with_registry!(
                "gas_pool_available_gas_total_balance",
                "Current total balance of available gas coins for reservation",
                &["sponsor"],
                registry,
            )
            .unwrap(),
            num_reserve_gas_coins_requests: register_int_counter_with_registry!(
                "num_reserve_gas_coins_requests",
                "Total number of reserve_gas_coins requests received",
                registry,
            )
            .unwrap(),
            num_successful_reserve_gas_coins_requests: register_int_counter_with_registry!(
                "num_successful_reserve_gas_coins_requests",
                "Total number of reserve_gas_coins requests that were successful",
                registry,
            )
            .unwrap(),
            num_ready_for_execution_requests: register_int_counter_with_registry!(
                "num_ready_for_execution_requests",
                "Total number of ready_for_execution requests received",
                registry,
            )
            .unwrap(),
            num_successful_ready_for_execution_requests: register_int_counter_with_registry!(
                "num_successful_ready_for_execution_requests",
                "Total number of ready_for_execution requests that were successful",
                registry,
            )
            .unwrap(),
            num_add_new_coins_requests: register_int_counter_with_registry!(
                "num_add_new_coins_requests",
                "Total number of add_new_coins requests received",
                registry,
            )
            .unwrap(),
            num_successful_add_new_coins_requests: register_int_counter_with_registry!(
                "num_successful_add_new_coins_requests",
                "Total number of add_new_coins requests that were successful",
                registry,
            )
            .unwrap(),
            num_expire_coins_requests: register_int_counter_with_registry!(
                "num_expire_coins_requests",
                "Total number of expire_coins requests received",
                registry,
            )
            .unwrap(),
            num_successful_expire_coins_requests: register_int_counter_with_registry!(
                "num_successful_expire_coins_requests",
                "Total number of expire_coins requests that were successful",
                registry,
            )
            .unwrap(),
        })
    }

    pub fn new_for_testing() -> Arc<Self> {
        Self::new(&Registry::new())
    }
}
