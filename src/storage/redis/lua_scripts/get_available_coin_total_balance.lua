-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to get the total balance of available gas coins for a sponsor address.
-- It is expensive to call this function since it iterates over all available coins and parse them.
-- The first argument is the sponsor's address

local sponsor_address = ARGV[1]

local t_available_gas_coins = sponsor_address .. ':available_gas_coins'

local elements = redis.call('LRANGE', t_available_gas_coins, 0, -1)

local total = 0
for i = 1, #elements do
    -- Each coin is just a string, using "," to separate fields. The first is balance.
    local coin = elements[i]
    local idx, _ = string.find(coin, ',', 1)
    local balance = string.sub(coin, 1, idx - 1)
    total = total + tonumber(balance)
end

return total
