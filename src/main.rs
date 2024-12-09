// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use sentry;
use std::env;
use std::{marker::PhantomData, str::FromStr, sync::Arc};
use sui_gas_pool::command::Command;

fn main() {
    let _guard = sentry::init(("https://a4d7d57ce55195bb2c8df9c2bb7e8154@o4507907608150016.ingest.de.sentry.io/4507929897140304", sentry::ClientOptions {
        release: sentry::release_name!(),
        attach_stacktrace: true,
        environment: Some(env::var("NODE_ENV").expect("NODE_ENV not found in environment variables").into()),
        before_send: Some(Arc::new(|event| {
            if event.environment.as_deref() == Some("development") {
                return None;
            }
            Some(event)
        })),
        ..Default::default()
      }));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let command = Command::parse();
            command.execute().await;
        });
}
