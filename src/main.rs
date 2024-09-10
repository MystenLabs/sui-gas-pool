// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use sui_gas_pool::command::Command;

fn main() {
    let _guard = sentry::init(("https://a4d7d57ce55195bb2c8df9c2bb7e8154@o4507907608150016.ingest.de.sentry.io/4507929897140304", sentry::ClientOptions {
        release: sentry::release_name!(),
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
