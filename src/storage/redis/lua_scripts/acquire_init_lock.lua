-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

-- Acquires a lock for initializing a sponsor's account.
-- We need a lock because during initialization we need to split the gas coins.
-- This cannot be done concurrently on multiple gas stations.
-- The lock is acquired for a certain duration, after which it is released.
-- The duration should be long enough such that the initialization can be completed.
-- If the lock is already acquired, the function returns 0.
-- If the lock is acquired, the function returns 1 and sets the lock's expiration time.
-- The first argument is the sponsor's address.
-- The second argument is the current timestamp.
-- The third argument is the duration for which the lock should be held. This should be in the same
-- units as the current timestamp.

local sponsor_address = ARGV[1]
local current_time = tonumber(ARGV[2])
local lock_duration = tonumber(ARGV[3])

local t_init_lock = sponsor_address .. ':init_lock'
local locked_timestamp = redis.call('GET', t_init_lock)

if locked_timestamp == false or tonumber(locked_timestamp) < current_time then
    redis.call('SET', t_init_lock, current_time + lock_duration)
    return 1
else
    return 0
end
