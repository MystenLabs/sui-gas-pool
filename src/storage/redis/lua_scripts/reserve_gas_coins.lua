-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]
local target_budget = tonumber(ARGV[2])
local expiration_time = tonumber(ARGV[3])

local MAX_GAS_PER_QUERY = 256

local t_available_gas_coin_balances = sponsor_address .. ':available_gas_coin_balances'
local t_available_gas_coin_object_ids = sponsor_address .. ':available_gas_coin_object_ids'
local t_available_gas_coin_object_versions = sponsor_address .. ':available_gas_coin_object_versions'
local t_available_gas_coin_object_digests = sponsor_address .. ':available_gas_coin_object_digests'
local t_expiration_queue = sponsor_address .. ':expiration_queue'
local t_next_reservation_id = sponsor_address .. ':next_reservation_id'

local total_balance = 0
local coin_count = 0
local balances = {}
local object_ids = {}
local object_versions = {}
local object_digests = {}

while total_balance < target_budget and coin_count < MAX_GAS_PER_QUERY do
    local balance = redis.call('LPOP', t_available_gas_coin_balances)
    if not balance then break end

    balance = tonumber(balance)
    total_balance = total_balance + balance
    coin_count = coin_count + 1

    local object_id = redis.call('LPOP', t_available_gas_coin_object_ids)
    local object_version = redis.call('LPOP', t_available_gas_coin_object_versions)
    local object_digest = redis.call('LPOP', t_available_gas_coin_object_digests)

    table.insert(balances, balance)
    table.insert(object_ids, object_id)
    table.insert(object_versions, object_version)
    table.insert(object_digests, object_digest)
end

if total_balance < target_budget then
    -- If the threshold is not reached, push the numbers back to the front of the queue in the original order.
    for i = #balances, 1, -1 do
        redis.call('LPUSH', t_available_gas_coin_balances, balances[i])
        redis.call('LPUSH', t_available_gas_coin_object_ids, object_ids[i])
        redis.call('LPUSH', t_available_gas_coin_object_versions, object_versions[i])
        redis.call('LPUSH', t_available_gas_coin_object_digests, object_digests[i])
    end
    return {0, {}, {}, {}, {}}
end

redis.call('INCR', t_next_reservation_id)
local reservation_id = redis.call('GET', t_next_reservation_id)
local concated_object_ids = table.concat(object_ids, ',')
local key = sponsor_address .. ':' .. reservation_id
redis.call('SET', key, concated_object_ids)
redis.call('ZADD', t_expiration_queue, expiration_time, reservation_id)

return {reservation_id, balances, object_ids, object_versions, object_digests}
