-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to add new coins to the available gas coins queue.
-- The first argument is the sponsor's address.
-- The second argument is a JSON array of new coins.
-- Each coin is just a string, using "," to separate these fields:
--   balance, object id, object version, object digest.
-- In this script we don't care about the format, just push each to the queue.
-- We also set the initialized flag to 1 if we added any coins.
-- Returns a table with the new total balance and new coin count.

local sponsor_address = ARGV[1]
local new_coins = ARGV[2]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local decoded_new_coins = cjson.decode(new_coins)
local count = #decoded_new_coins

local total_balance = 0
for i = 1, count, 1 do
    local coin = decoded_new_coins[i]
    local idx1, _ = string.find(coin, ',', 1)
    local balance = string.sub(coin, 1, idx1 - 1)
    total_balance = total_balance + tonumber(balance)

    redis.call('RPUSH', t_available_gas_coins, coin)
end

if count > 0 then
	local initialized_key = sponsor_address .. ':initialized'
    redis.call('SET', initialized_key, 1)
end

local t_available_coin_total_balance = sponsor_address .. ':available_coin_total_balance'
-- TODO: For some reason INCRBY is not working, so we have to do this in two steps.
local cur_coin_total_balance = tonumber(redis.call('GET', t_available_coin_total_balance)) or 0
local new_total_balance = cur_coin_total_balance + total_balance
-- Use string.format to avoid scientific notation
redis.call('SET', t_available_coin_total_balance, string.format("%.0f", new_total_balance))

local t_available_coin_count = sponsor_address .. ':available_coin_count'
local cur_coin_count = tonumber(redis.call('GET', t_available_coin_count)) or 0
local new_coin_count = cur_coin_count + count
redis.call('SET', t_available_coin_count, new_coin_count)

return {new_total_balance, new_coin_count}
