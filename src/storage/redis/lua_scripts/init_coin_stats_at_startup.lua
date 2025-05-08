-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to initialize a few coin related statistics for a sponsor address at startup.
-- Including the total balance and the total coin count.
-- The first argument is the sponsor's address.
-- Returns a table with the new coin count and new total balance.

local sponsor_address = ARGV[1]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local t_available_coin_count = sponsor_address .. ':available_coin_count'
local coin_count = redis.call('GET', t_available_coin_count)
if not coin_count then
    coin_count = redis.call('LLEN', t_available_gas_coins)
    redis.call('SET', t_available_coin_count, coin_count)
end

local t_available_coin_total_balance = sponsor_address .. ':available_coin_total_balance'
local total_balance = redis.call('GET', t_available_coin_total_balance)
redis.log(redis.LOG_WARNING, "total_balance before if: " .. tostring(total_balance))

if not total_balance then
    local elements = redis.call('LRANGE', t_available_gas_coins, 0, -1)
    total_balance = 0
    for _, coin in ipairs(elements) do
        -- Each coin is just a string, using "," to separate fields. The first is balance.
        local idx, _ = string.find(coin, ',', 1)
        local balance = string.sub(coin, 1, idx - 1)
        redis.log(redis.LOG_WARNING, "parsed balance: " .. balance)
        -- Handle scientific notation by converting to a regular number
        total_balance = total_balance + math.tointeger(balance)
        redis.log(redis.LOG_WARNING, "updated total_balance: " .. total_balance)
    end
    redis.call('SET', t_available_coin_total_balance, total_balance)
end

return {coin_count, total_balance}
