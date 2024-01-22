use once_cell::sync::Lazy;
use redis::Script;

const RESERVE_GAS_COINS_SCRIPT: &str = include_str!("lua_scripts/reserve_gas_coins.lua");
const ADD_NEW_COINS_SCRIPT: &str = include_str!("lua_scripts/add_new_coins.lua");
const READY_FOR_EXECUTION_SCRIPT: &str = include_str!("lua_scripts/ready_for_execution.lua");
const EXPIRE_COINS_SCRIPT: &str = include_str!("lua_scripts/expire_coins.lua");
const GET_AVAILABLE_COIN_COUNT_SCRIPT: &str =
    include_str!("lua_scripts/get_available_coin_count.lua");
#[cfg(test)]
const GET_AVAILABLE_COIN_TOTAL_BALANCE_SCRIPT: &str =
    include_str!("lua_scripts/get_available_coin_total_balance.lua");
#[cfg(test)]
const GET_RESERVED_COIN_COUNT_SCRIPT: &str =
    include_str!("lua_scripts/get_reserved_coin_count.lua");

pub struct ScriptManager;

impl ScriptManager {
    pub fn reserve_gas_coins_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(RESERVE_GAS_COINS_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    pub fn add_new_coins_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(ADD_NEW_COINS_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    pub fn ready_for_execution_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(READY_FOR_EXECUTION_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    pub fn expire_coins_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(EXPIRE_COINS_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    pub fn get_available_coin_count_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(GET_AVAILABLE_COIN_COUNT_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    // This needs to be test only because it's really expensive to call in production.
    #[cfg(test)]
    pub fn get_available_coin_total_balance_script() -> &'static Script {
        static SCRIPT: Lazy<Script> =
            Lazy::new(|| Script::new(GET_AVAILABLE_COIN_TOTAL_BALANCE_SCRIPT));
        Lazy::force(&SCRIPT)
    }

    // This needs to be test only because it's really expensive to call in production.
    #[cfg(test)]
    pub fn get_reserved_coin_count_script() -> &'static Script {
        static SCRIPT: Lazy<Script> = Lazy::new(|| Script::new(GET_RESERVED_COIN_COUNT_SCRIPT));
        Lazy::force(&SCRIPT)
    }
}
