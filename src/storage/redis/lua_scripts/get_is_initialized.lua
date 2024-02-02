-- Copyright (c) Mysten Labs, Inc.
-- SPDX-License-Identifier: Apache-2.0

local sponsor_address = ARGV[1]

local initialized_key = sponsor_address .. ':initialized'
local exists = redis.call('EXISTS', initialized_key)
return exists
