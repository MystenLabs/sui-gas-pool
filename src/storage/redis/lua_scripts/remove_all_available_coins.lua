-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'
local t_expiration_queue = sponsor_address .. ':expiration_queue'

while (redis.call('LPOP', t_available_gas_coins)) do
end

redis.call('ZREMRANGEBYSCORE', t_expiration_queue, '-inf', '+inf')
