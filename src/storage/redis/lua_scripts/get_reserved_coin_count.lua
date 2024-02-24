-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- This script is used to get the number of reserved gas coins for a sponsor address.
-- It is used for debugging purpose only.
-- The first argument is the sponsor's address.

local sponsor_address = ARGV[1]

local t_expiration_queue = sponsor_address .. ':expiration_queue'

local elements = redis.call('ZRANGE', t_expiration_queue, 0, -1)

local total = 0
for _, reservation_id in ipairs(elements) do
    local key = sponsor_address .. ':' .. reservation_id
    local object_ids = redis.call('GET', key)
    if object_ids then
        -- The object IDs are concatenated with commas. gsub will replace all commas with empty strings and
        -- return the number of replacements. We add 1 to the count to get the number of object IDs.
        local _, count = string.gsub(object_ids, ',', '')
        total = total + count + 1
    end
end

return total
