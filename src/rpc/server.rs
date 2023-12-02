// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_station::simple_gas_station::SimpleGasStation;
use crate::gas_station::GasStation;
use crate::rpc::rpc_types::{ReleaseGasRequest, ReserveGasRequest, ReserveGasResponse};
use crate::types::GaslessTransaction;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use sui_types::transaction::TransactionExpiration;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub struct GasStationServer {
    pub handle: JoinHandle<()>,
}

impl GasStationServer {
    pub async fn new(station: Arc<SimpleGasStation>, rpc_port: u16) -> Self {
        let app = Router::new()
            .route("/", get(health))
            .route("/v1/reserve_gas", post(reserve_gas))
            .route("/v1/release_gas", post(release_gas))
            .layer(Extension(station));
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rpc_port);
        let handle = tokio::spawn(async move {
            info!("listening on {}", address);
            axum::Server::bind(&address)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        Self { handle }
    }
}

async fn health() -> &'static str {
    info!("Received health request");
    "OK"
}

async fn reserve_gas(
    Extension(server): Extension<Arc<SimpleGasStation>>,
    Json(payload): Json<ReserveGasRequest>,
) -> impl IntoResponse {
    debug!("Received v1 reserve_gas request: {:?}", payload);
    let ReserveGasRequest {
        programmable,
        sender,
        expiration,
        budget,
        price,
        reserve_duration_secs,
    } = payload;
    let expiration = match expiration {
        Some(expiration) => TransactionExpiration::Epoch(expiration),
        None => TransactionExpiration::None,
    };
    let tx = GaslessTransaction {
        pt: programmable,
        sender,
        expiration,
        budget,
        price,
    };
    match server
        .reserve_gas(tx, Duration::from_secs(reserve_duration_secs))
        .await
    {
        Ok((transaction, signature)) => {
            let response = ReserveGasResponse {
                tx_and_sig: Some((transaction, signature)),
                error: None,
            };
            (StatusCode::OK, Json(response))
        }
        Err(err) => (
            StatusCode::NO_CONTENT,
            Json(ReserveGasResponse {
                tx_and_sig: None,
                error: Some(err.to_string()),
            }),
        ),
    }
}

async fn release_gas(
    Extension(server): Extension<Arc<SimpleGasStation>>,
    Json(payload): Json<ReleaseGasRequest>,
) -> impl IntoResponse {
    debug!("Received v1 release_gas request: {:?}", payload);
    let ReleaseGasRequest { gas_coins } = payload;
    server.release_gas(gas_coins).await;
    (StatusCode::OK, Json(()))
}
