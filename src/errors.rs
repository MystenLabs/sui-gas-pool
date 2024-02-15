// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

#[macro_export]
macro_rules! retry_with_max_attempts {
    ($func:expr, $max_attempts:expr) => {{
        let retry_strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(50)
            .max_delay(std::time::Duration::from_secs(1))
            .take($max_attempts)
            .map(tokio_retry::strategy::jitter);
        tokio_retry::Retry::spawn(retry_strategy, || $func).await
    }};
}

#[cfg(not(test))]
#[macro_export]
macro_rules! retry_forever {
    ($func:expr) => {{
        let retry_strategy = tokio_retry::strategy::FixedInterval::from_millis(500);
        tokio_retry::Retry::spawn(retry_strategy, || $func).await
    }};
}

#[cfg(test)]
#[macro_export]
macro_rules! retry_forever {
    ($func:expr) => {{
        $func.await
    }};
}
