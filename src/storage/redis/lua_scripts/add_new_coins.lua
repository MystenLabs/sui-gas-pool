-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]
-- new_coins is a JSON array of new coins.
-- Each coin is just a string, using "," to separate these fields:
--   balance, object id, object version, object digest.
-- Here we don't care about the format, just push each to the queue.
local new_coins = ARGV[2]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local decoded_new_coins = cjson.decode(new_coins)

for i = 1, #decoded_new_coins, 1 do
    redis.call('RPUSH', t_available_gas_coins, decoded_new_coins[i])
end
