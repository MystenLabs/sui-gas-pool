-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local length = redis.call('LLEN', t_available_gas_coins)
return length
