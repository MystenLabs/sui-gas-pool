// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::bail;
use chrono::{Local, NaiveDate};
use tokio::sync::RwLock;

pub struct GasUsageCap {
    daily_cap: u64,
    inner: RwLock<GasUsageCapInner>,
}

struct GasUsageCapInner {
    cur_daily_usage: i64,
    cur_date: NaiveDate,
}

impl GasUsageCap {
    pub fn new(daily_cap: u64) -> Self {
        Self {
            daily_cap,
            inner: RwLock::new(GasUsageCapInner {
                cur_daily_usage: 0,
                cur_date: Local::now().date_naive(),
            }),
        }
    }

    pub async fn check_usage(&self) -> anyhow::Result<()> {
        self.reset_date_maybe().await;
        let cur_daily_usage = self.inner.read().await.cur_daily_usage;
        if cur_daily_usage >= self.daily_cap as i64 {
            bail!("Gas usage exceeds daily cap");
        }
        Ok(())
    }

    /// Update daily usage and returns the new current usage.
    pub async fn update_usage(&self, usage: i64) -> i64 {
        self.reset_date_maybe().await;
        let mut inner = self.inner.write().await;
        inner.cur_daily_usage += usage;
        inner.cur_daily_usage
    }

    async fn reset_date_maybe(&self) {
        let today = Local::now().date_naive();
        let cur_date = self.inner.read().await.cur_date;
        if cur_date != today {
            self.inner.write().await.reset_date(today);
        }
    }
}

impl GasUsageCapInner {
    fn reset_date(&mut self, new_date: NaiveDate) {
        self.cur_date = new_date;
        self.cur_daily_usage = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gas_usage_cap() {
        let cap = GasUsageCap::new(100);
        assert!(cap.check_usage().await.is_ok());
        cap.update_usage(50).await;
        assert!(cap.check_usage().await.is_ok());
        cap.update_usage(49).await;
        assert!(cap.check_usage().await.is_ok());
        cap.update_usage(1).await;
        assert!(cap.check_usage().await.is_err());
    }

    #[tokio::test]
    async fn test_gas_usage_cap_reset() {
        let today = Local::now().date_naive();
        let cap = GasUsageCap::new(100);
        cap.update_usage(100).await;
        assert!(cap.check_usage().await.is_err());
        cap.inner.write().await.cur_date = today + chrono::Duration::days(1);
        assert!(cap.check_usage().await.is_ok());
    }
}
