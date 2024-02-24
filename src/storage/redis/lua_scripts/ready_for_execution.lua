-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to mark a reservation as ready for execution.
-- It takes out the reservation from the sponsor's reservation map.
-- We need this such that a concurrent task that calls expire_coins.lua does not expire the same reservation again
-- right before the transaction is executed.
-- The first argument is the sponsor's address.
-- The second argument is the reservation id.

local sponsor_address = ARGV[1]
local reservation_id = ARGV[2]

local key = sponsor_address .. ':' .. reservation_id
local exists = redis.call('EXISTS', key)
if exists == 1 then
    redis.call('DEL', key)
else
    error('Reservation no longer exist: ' .. reservation_id)
end