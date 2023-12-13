// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_station::gas_station_core::{GasStation, GasStationContainer};
use crate::rpc::client::GasStationRpcClient;
use crate::rpc::rpc_types::{
    ExecuteTxRequest, ExecuteTxResponse, ReserveGasRequest, ReserveGasResponse,
};
use crate::test_env::start_gas_station;
use crate::{read_auth_env, AUTH_ENV_NAME};
use axum::headers::authorization::Bearer;
use axum::headers::Authorization;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router, TypedHeader};
use fastcrypto::encoding::Base64;
use fastcrypto::traits::ToFromBytes;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use sui_config::local_ip_utils::{get_available_port, localhost_for_testing};
use sui_types::signature::GenericSignature;
use sui_types::transaction::TransactionData;
use test_cluster::TestCluster;
use tokio::task::JoinHandle;
use tracing::{debug, info};

// TODO: Check what happens when client drops, and whether we need to persist the thread.

pub struct GasStationServer {
    pub handle: JoinHandle<()>,
    pub rpc_port: u16,
}

impl GasStationServer {
    pub async fn new(station: Arc<GasStation>, host_ip: Ipv4Addr, rpc_port: u16) -> Self {
        let state = ServerState::new(station);
        let app = Router::new()
            .route("/", get(health))
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

    pub fn get_local_client(&self) -> GasStationRpcClient {
        GasStationRpcClient::new(format!("http://localhost:{}", self.rpc_port))
    }

    pub async fn start_rpc_server_for_testing(
        init_gas_amounts: Vec<u64>,
    ) -> (TestCluster, GasStationContainer, GasStationServer) {
        let (test_cluster, container) = start_gas_station(init_gas_amounts).await;
        let localhost = localhost_for_testing();
        std::env::set_var(AUTH_ENV_NAME, "some secret");
        let server = GasStationServer::new(
            container.get_station(),
            localhost.parse().unwrap(),
            get_available_port(&localhost),
        )
        .await;
        (test_cluster, container, server)
    }
}

#[derive(Clone)]
struct ServerState {
    gas_station: Arc<GasStation>,
    secret: Arc<String>,
}

impl ServerState {
    fn new(gas_station: Arc<GasStation>) -> Self {
        let secret = Arc::new(read_auth_env());
        Self {
            gas_station,
            secret,
        }
    }
}

async fn health() -> &'static str {
    info!("Received health request");
    "OK"
}

async fn reserve_gas(
    TypedHeader(authorization): TypedHeader<Authorization<Bearer>>,
    Extension(server): Extension<ServerState>,
    Json(payload): Json<ReserveGasRequest>,
) -> impl IntoResponse {
    debug!("Received v1 reserve_gas request: {:?}", payload);
    if authorization.token() != server.secret.as_str() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ReserveGasResponse::new_err(anyhow::anyhow!(
                "Invalid authorization token"
            ))),
        );
    }
    let ReserveGasRequest {
        gas_budget,
        request_sponsor,
        reserve_duration_secs,
    } = payload;
    match server
        .gas_station
        .reserve_gas(
            request_sponsor,
            gas_budget,
            Duration::from_secs(reserve_duration_secs),
        )
        .await
    {
        Ok((sponsor, gas_coins)) => {
            let response = ReserveGasResponse::new_ok(sponsor, gas_coins);
            (StatusCode::OK, Json(response))
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ReserveGasResponse::new_err(err)),
        ),
    }
}

async fn execute_tx(
    TypedHeader(authorization): TypedHeader<Authorization<Bearer>>,
    Extension(server): Extension<ServerState>,
    Json(payload): Json<ExecuteTxRequest>,
) -> impl IntoResponse {
    debug!("Received v1 execute_tx request: {:?}", payload);
    if authorization.token() != server.secret.as_ref() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ExecuteTxResponse::new_err(anyhow::anyhow!(
                "Invalid authorization token"
            ))),
        );
    }
    let ExecuteTxRequest { tx_bytes, user_sig } = payload;
    let Ok((tx_data, user_sig)) = convert_tx_and_sig(tx_bytes, user_sig) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ExecuteTxResponse::new_err(anyhow::anyhow!(
                "Invalid bcs bytes for TransactionData"
            ))),
        );
    };
    // TODO: Should we check user signature?
    match server
        .gas_station
        .execute_transaction(tx_data, user_sig)
        .await
    {
        Ok(effects) => (StatusCode::OK, Json(ExecuteTxResponse::new_ok(effects))),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ExecuteTxResponse::new_err(err)),
        ),
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
