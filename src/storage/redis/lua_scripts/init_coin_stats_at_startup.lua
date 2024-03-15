-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to initialize a few coin related statistics for a sponsor address at startup.
-- Including the total balance and the total coin count.
-- The first argument is the sponsor's address.
-- Returns a table with the new coin count and new total balance.

local sponsor_address = ARGV[1]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'
local coin_count = redis.call('LLEN', t_available_gas_coins)
local t_available_coin_count = sponsor_address .. ':available_coin_count'
redis.call('SET', t_available_coin_count, coin_count)

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'
local elements = redis.call('LRANGE', t_available_gas_coins, 0, -1)
local total_balance = 0
for i = 1, #elements do
    -- Each coin is just a string, using "," to separate fields. The first is balance.
    local coin = elements[i]
    local idx, _ = string.find(coin, ',', 1)
    local balance = string.sub(coin, 1, idx - 1)
    total_balance = total_balance + tonumber(balance)
end
local t_available_coin_total_balance = sponsor_address .. ':available_coin_total_balance'
redis.call('SET', t_available_coin_total_balance, total_balance)

return {coin_count, total_balance}
