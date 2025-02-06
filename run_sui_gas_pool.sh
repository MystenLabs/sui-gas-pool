#!/bin/bash

# Start haproxy in background
/usr/sbin/haproxy -f /etc/haproxy/haproxy.cfg

# Start promtail in background
/usr/local/bin/promtail -config.file=/etc/promtail/config.yml &

# Start sui gas pool
/usr/local/bin/sui_gas_pool --config-path /usr/local/bin/mainnet.yaml