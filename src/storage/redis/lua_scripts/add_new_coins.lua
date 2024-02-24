-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to add new coins to the available gas coins queue.
-- The first argument is the sponsor's address.
-- The second argument is a JSON array of new coins.
-- Each coin is just a string, using "," to separate these fields:
--   balance, object id, object version, object digest.
-- In this script we don't care about the format, just push each to the queue.
-- We also set the initialized flag to 1 if we added any coins.

local sponsor_address = ARGV[1]
local new_coins = ARGV[2]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local decoded_new_coins = cjson.decode(new_coins)
local count = #decoded_new_coins

for i = 1, count, 1 do
    redis.call('RPUSH', t_available_gas_coins, decoded_new_coins[i])
end

if count > 0 then
	local initialized_key = sponsor_address .. ':initialized'
    redis.call('SET', initialized_key, 1)
end
