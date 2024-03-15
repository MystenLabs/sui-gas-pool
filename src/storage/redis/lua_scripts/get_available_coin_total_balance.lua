-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to get the total balance of available gas coins for a sponsor address.
-- The first argument is the sponsor's address

local sponsor_address = ARGV[1]

local t_available_coin_total_balance = sponsor_address .. ':available_coin_total_balance'
local total_balance = redis.call('GET', t_available_coin_total_balance)

return total_balance
