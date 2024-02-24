-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to reserve gas coins for a sponsor address.
-- It takes out gas coins from the available_gas_coins list and returns them to the caller.
-- It also creates a unique reservation id and stores the reserved coins in a separate reservation map.
-- The reservation id is used to track the reserved coins and to release them back to the available pool if not used.
-- The reservation id is added to the expiration_queue to track the expiration time of the reserved coins.
-- The first argument is the sponsor's address.
-- The second argument is the target budget.
-- The third argument is the expiration time.

local sponsor_address = ARGV[1]
local target_budget = tonumber(ARGV[2])
local expiration_time = tonumber(ARGV[3])

local MAX_GAS_PER_QUERY = 256

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'
local t_expiration_queue = sponsor_address .. ':expiration_queue'
local t_next_reservation_id = sponsor_address .. ':next_reservation_id'

local total_balance = 0
local coin_count = 0
local coins = {}
local object_ids = {}

while total_balance < target_budget and coin_count < MAX_GAS_PER_QUERY do
    local coin = redis.call('LPOP', t_available_gas_coins)
    if not coin then break end

    local idx1, _ = string.find(coin, ',', 1)
    local balance = string.sub(coin, 1, idx1 - 1)
    total_balance = total_balance + tonumber(balance)
    coin_count = coin_count + 1

    local idx2, _ = string.find(coin, ',', idx1 + 1)
    local object_id = string.sub(coin, idx1 + 1, idx2 - 1)

    table.insert(coins, coin)
    table.insert(object_ids, object_id)
end

if total_balance < target_budget then
    -- If the threshold is not reached, push the coins back to the front of the queue in the original order.
    for i = #coins, 1, -1 do
        redis.call('LPUSH', t_available_gas_coins, coins[i])
    end
    return {0, {}}
end

redis.call('INCR', t_next_reservation_id)
local reservation_id = redis.call('GET', t_next_reservation_id)
local concated_object_ids = table.concat(object_ids, ',')
local key = sponsor_address .. ':' .. reservation_id
redis.call('SET', key, concated_object_ids)
redis.call('ZADD', t_expiration_queue, expiration_time, reservation_id)

return {reservation_id, coins}
