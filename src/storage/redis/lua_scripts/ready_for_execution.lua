-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]
local reservation_id = ARGV[2]

local key = sponsor_address .. ':' .. reservation_id
local exists = redis.call('EXISTS', key)
if exists == 1 then
    redis.call('DEL', key)
    return 1
else
    return 0
end