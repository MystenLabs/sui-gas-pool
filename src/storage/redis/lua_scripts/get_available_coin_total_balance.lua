-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]

local t_available_gas_coin_balances = sponsor_address .. ':available_gas_coin_balances'

local elements = redis.call('LRANGE', t_available_gas_coin_balances, 0, -1)

local total = 0
for i = 1, #elements do
    total = total + tonumber(elements[i])
end

return total
