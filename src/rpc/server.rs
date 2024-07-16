// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_pool::gas_pool_core::GasPool;
use crate::metrics::GasPoolRpcMetrics;
use crate::read_auth_env;
use crate::rpc::client::GasPoolRpcClient;
use crate::rpc::rpc_types::{
    ExecuteTxRequest, ExecuteTxResponse, ReserveGasRequest, ReserveGasResponse,
};
use axum::headers::authorization::Bearer;
use axum::headers::Authorization;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router, TypedHeader};
use fastcrypto::encoding::Base64;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
use sui_types::crypto::ToFromBytes;
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

const GIT_REVISION: &str = {
    if let Some(revision) = option_env!("GIT_REVISION") {
        revision
    } else {
        let version = git_version::git_version!(
            args = ["--always", "--abbrev=12", "--dirty", "--exclude", "*"],
            fallback = ""
        );

        if version.is_empty() {
            panic!("unable to query git revision");
        }
        version
    }
};
const VERSION: &str = const_str::concat!(env!("CARGO_PKG_VERSION"), "-", GIT_REVISION);

pub struct GasPoolServer {
    pub handle: JoinHandle<()>,
    pub rpc_port: u16,
}

impl GasPoolServer {
    pub async fn new(
        station: Arc<GasPool>,
        host_ip: Ipv4Addr,
        rpc_port: u16,
        metrics: Arc<GasPoolRpcMetrics>,
    ) -> Self {
        let state = ServerState::new(station, metrics);
        let app = Router::new()
            .route("/", get(health))
            .route("/version", get(version))
            .route("/debug_health_check", post(debug_health_check))
            .route("/v1/reserve_gas", post(reserve_gas))
            .route("/v1/execute_tx", post(execute_tx))
            .layer(Extension(state));
        let address = SocketAddr::new(IpAddr::V4(host_ip), rpc_port);
        let handle = tokio::spawn(async move {
            info!("listening on {}", address);
            axum::Server::bind(&address)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        Self { handle, rpc_port }
    }

    pub fn get_local_client(&self) -> GasPoolRpcClient {
        GasPoolRpcClient::new(format!("http://localhost:{}", self.rpc_port))
    }
}

#[derive(Clone)]
struct ServerState {
    gas_station: Arc<GasPool>,
    secret: Arc<String>,
    metrics: Arc<GasPoolRpcMetrics>,
}

impl ServerState {
    fn new(gas_station: Arc<GasPool>, metrics: Arc<GasPoolRpcMetrics>) -> Self {
        let secret = Arc::new(read_auth_env());
        Self {
            gas_station,
            secret,
            metrics,
        }
    }
}

async fn health() -> &'static str {
    info!("Received health request");
    "OK"
}

async fn version() -> &'static str {
    info!("Received version request");
    VERSION
}

async fn debug_health_check(
    TypedHeader(authorization): TypedHeader<Authorization<Bearer>>,
    Extension(server): Extension<ServerState>,
) -> String {
    info!("Received debug_health_check request");
    if authorization.token() != server.secret.as_str() {
        return "Unauthorized".to_string();
    }
    if let Err(err) = server.gas_station.debug_check_health().await {
        return format!("Failed to check health: {:?}", err);
    }
    "OK".to_string()
}

async fn reserve_gas(
    TypedHeader(authorization): TypedHeader<Authorization<Bearer>>,
    Extension(server): Extension<ServerState>,
    Json(payload): Json<ReserveGasRequest>,
) -> impl IntoResponse {
    server.metrics.num_reserve_gas_requests.inc();
    if authorization.token() != server.secret.as_str() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ReserveGasResponse::new_err(anyhow::anyhow!(
                "Invalid authorization token"
            ))),
        );
    }
    server.metrics.num_authorized_reserve_gas_requests.inc();
    debug!("Received v1 reserve_gas request: {:?}", payload);
    if let Err(err) = payload.check_validity() {
        debug!("Invalid reserve_gas request: {:?}", err);
        return (
            StatusCode::BAD_REQUEST,
            Json(ReserveGasResponse::new_err(err)),
        );
    }
    let ReserveGasRequest {
        gas_budget,
        reserve_duration_secs,
    } = payload;
    server
        .metrics
        .target_gas_budget_per_request
        .observe(gas_budget);
    server
        .metrics
        .reserve_duration_per_request
        .observe(reserve_duration_secs);
    // Spawn a thread to process the request so that it will finish even when client drops the connection.
    tokio::task::spawn(reserve_gas_impl(
        server.gas_station.clone(),
        server.metrics.clone(),
        gas_budget,
        reserve_duration_secs,
    ))
    .await
    .unwrap_or_else(|err| {
        error!("Failed to spawn reserve_gas task: {:?}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ReserveGasResponse::new_err(anyhow::anyhow!(
                "Failed to spawn reserve_gas task"
            ))),
        )
    })
}

async fn reserve_gas_impl(
    gas_station: Arc<GasPool>,
    metrics: Arc<GasPoolRpcMetrics>,
    gas_budget: u64,
    reserve_duration_secs: u64,
) -> (StatusCode, Json<ReserveGasResponse>) {
    match gas_station
        .reserve_gas(gas_budget, Duration::from_secs(reserve_duration_secs))
        .await
    {
        Ok((sponsor, reservation_id, gas_coins)) => {
            info!(
                ?reservation_id,
                "Reserved gas coins with sponsor={:?}, budget={:?} and duration={:?}: {:?}",
                sponsor,
                gas_budget,
                reserve_duration_secs,
                gas_coins
            );
            metrics.num_successful_reserve_gas_requests.inc();
            let response = ReserveGasResponse::new_ok(sponsor, reservation_id, gas_coins);
            (StatusCode::OK, Json(response))
        }
        Err(err) => {
            error!("Failed to reserve gas: {:?}", err);
            metrics.num_failed_reserve_gas_requests.inc();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ReserveGasResponse::new_err(err)),
            )
        }
    }
}

async fn execute_tx(
    TypedHeader(authorization): TypedHeader<Authorization<Bearer>>,
    Extension(server): Extension<ServerState>,
    Json(payload): Json<ExecuteTxRequest>,
) -> impl IntoResponse {
    server.metrics.num_execute_tx_requests.inc();
    if authorization.token() != server.secret.as_ref() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ExecuteTxResponse::new_err(anyhow::anyhow!(
                "Invalid authorization token"
            ))),
        );
    }
    server.metrics.num_authorized_execute_tx_requests.inc();
    debug!("Received v1 execute_tx request: {:?}", payload);
    let ExecuteTxRequest {
        reservation_id,
        tx_bytes,
        user_sig,
    } = payload;
    let Ok((tx_data, user_sig)) = convert_tx_and_sig(tx_bytes, user_sig) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ExecuteTxResponse::new_err(anyhow::anyhow!(
                "Invalid bcs bytes for TransactionData"
            ))),
        );
    };
    // Spawn a thread to process the request so that it will finish even when client drops the connection.
    tokio::task::spawn(execute_tx_impl(
        server.gas_station.clone(),
        server.metrics.clone(),
        reservation_id,
        tx_data,
        user_sig,
    ))
    .await
    .unwrap_or_else(|err| {
        error!("Failed to spawn reserve_gas task: {:?}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ExecuteTxResponse::new_err(anyhow::anyhow!(
                "Failed to spawn execute_tx task"
            ))),
        )
    })
}

async fn execute_tx_impl(
    gas_station: Arc<GasPool>,
    metrics: Arc<GasPoolRpcMetrics>,
    reservation_id: u64,
    tx_data: TransactionData,
    user_sig: GenericSignature,
) -> (StatusCode, Json<ExecuteTxResponse>) {
    match gas_station
        .execute_transaction(reservation_id, tx_data, user_sig)
        .await
    {
        Ok(response) => {
            info!(
                ?reservation_id,
                "Successfully executed transaction {:?} with status: {:?}",
                response.effects.clone().unwrap().transaction_digest(),
                response.effects.clone().unwrap().status()
            );
            metrics.num_successful_execute_tx_requests.inc();
            (StatusCode::OK, Json(ExecuteTxResponse::new_ok(response)))
        }
        Err(err) => {
            error!("Failed to execute transaction: {:?}", err);
            metrics.num_failed_execute_tx_requests.inc();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecuteTxResponse::new_err(err)),
            )
        }
    }
}

fn convert_tx_and_sig(
    tx_bytes: Base64,
    user_sig: Base64,
) -> anyhow::Result<(TransactionData, GenericSignature)> {
    let tx = bcs::from_bytes(
        &tx_bytes
            .to_vec()
            .map_err(|_| anyhow::anyhow!("Failed to convert tx_bytes to vector"))?,
    )?;
    let user_sig = GenericSignature::from_bytes(
        &user_sig
            .to_vec()
            .map_err(|_| anyhow::anyhow!("Failed to convert user_sig to vector"))?,
    )?;
    Ok((tx, user_sig))
}
