-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to get the number of available gas coins for a sponsor address.
-- The first argument is the sponsor's address.

local sponsor_address = ARGV[1]

local t_available_coin_count = sponsor_address .. ':available_coin_count'
local count = redis.call('GET', t_available_coin_count)

return count
