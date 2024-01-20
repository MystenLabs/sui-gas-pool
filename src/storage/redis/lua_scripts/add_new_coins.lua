-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]
local new_coins_balances = ARGV[2]
local new_coins_object_ids = ARGV[3]
local new_coins_object_versions = ARGV[4]
local new_coins_object_digests = ARGV[5]

local t_available_gas_coin_balances = sponsor_address .. ':available_gas_coin_balances'
local t_available_gas_coin_object_ids = sponsor_address .. ':available_gas_coin_object_ids'
local t_available_gas_coin_object_versions = sponsor_address .. ':available_gas_coin_object_versions'
local t_available_gas_coin_object_digests = sponsor_address .. ':available_gas_coin_object_digests'

local decoded_new_coins_balances = cjson.decode(new_coins_balances)
local decoded_new_coins_object_ids = cjson.decode(new_coins_object_ids)
local decoded_new_coins_object_versions = cjson.decode(new_coins_object_versions)
local decoded_new_coins_object_digests = cjson.decode(new_coins_object_digests)

for i = 1, #decoded_new_coins_balances, 1 do
    redis.call('RPUSH', t_available_gas_coin_balances, decoded_new_coins_balances[i])
    redis.call('RPUSH', t_available_gas_coin_object_ids, decoded_new_coins_object_ids[i])
    redis.call('RPUSH', t_available_gas_coin_object_versions, decoded_new_coins_object_versions[i])
    redis.call('RPUSH', t_available_gas_coin_object_digests, decoded_new_coins_object_digests[i])
end
