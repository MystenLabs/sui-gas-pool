-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]
local current_time = tonumber(ARGV[2])

local t_expiration_queue = sponsor_address .. ':expiration_queue'

local elements = redis.call('ZRANGEBYSCORE', t_expiration_queue, 0, current_time)

local expired_reservations = {}
if #elements > 0 then
    for _, reservation_id in ipairs(elements) do
        local key = sponsor_address .. ':' .. reservation_id
        local object_ids = redis.call('GET', key)
        if object_ids then
            redis.call('DEL', key)
            table.insert(expired_reservations, object_ids)
        end
    end
    redis.call('ZREMRANGEBYSCORE', t_expiration_queue, 0, current_time)
end

return expired_reservations
