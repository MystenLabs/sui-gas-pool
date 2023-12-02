# Enoki Gas Pool
This is a high-level summary of how Sui Gas Station works.

## Assumptions
1. The Gas Station can only be reached from an internal server, which handles rate limiting and authentication. The Gas Station blindly trusts the requests it receives.
2. The Gas Station relies on the caller to keep track the balance of the gas coins requested when returning them.

## Architecture
There are 5 components in the Gas Station:
### Storage
The storage layer stores the global gas pool information.
Any implementation must support the following two API functions:
- `fn reserve_gas_coins(&self, target_budget: u64) -> Vec<GasCoin>;`
- `fn add_gas_coins(&self, gas_coins: Vec<GasCoin>);`
Where `GasCoin` is defined as:
```rust
pub struct GasCoin {
    pub object_ref: ObjectRef,
    pub balance: u64,
}
```
`reserve_gas_coins` is able to take off potentially more than 1 gas coins from an internal queue atomically until the total accumulated balance reaches `target_budget`, or give up if it cannot find enough coins to meet the requirement even after searching for N coins (N can be implementation-specific, but must be less than 256 since it's our gas payment size cap).
This also takes advantage of the fact that gas smashing is able to merge gas coins during transaction execution. So if we have coins that have low balance, by returning multiple of them we are doing automatic dust collection.

`add_gas_coins` is able to add back gas coins to the gas pool. It can be called both when we are returning gas coins after using them, or adding new coins to the pool.

The storage layer is also expected to keep track the list of coins that are currently reserved. This is important because although rare, the gas station server may crash after reserving a coin and never come back up. This means over time some coins will be reserved but never released. If we have a table in the storage that keeps track of reserved coins along with their reservation time, we can then run a GC process from time to time to recollect gas coins that were reserved a long time ago.

*TODO*:
1. The current storage only has one implementation in RocksDB. We need to add a Redis implementation for it.
2. The current API and implementation doesn't support having multiple sponsor addresses, which we should add before going online.

### Gas Station Core
The Gas Station Core implements the core gas station logic that is able to process RPC requests and communicate with the Storage layer.
It has the following features:
1. After requesting gas coins from the Storage layer, it is able to persist that information to a local database, such that if it ever crashes, it can still remember the list of reserved coins after recovery.
2. It's able to automatically return reserved gas coins to the storage after the requested duration expires.
3. Once it obtains gas coins from the Storage layer, it then is able to construct a transaction data with gas payment set, and signs it with the sponsor private key.

*TODO*:
1. The automatic expiration needs to be implemented.
2. Key management needs to be more secure. Currently it just keeps the private key in memory, loaded from a config file.

### Gas Pool Initializer
A Gas Pool Initializer is able to initialize the global gas pool.
There are currently two implementations depending on the use case:
#### Production Initializer
This is what we will use in prod. When we are setting up the gas pool, we will need to run the `ProductionInitializer` exactly once. It is able to look at all the SUI coins currently owned by the sponsor address, and split them into gas coins with a specified target balance. This requires executing transactions hence it requires access to a fullnode (*TODO: This requires exposing it to external network. Is this safe?*).

*TODO*:
1. The current coin splitting code may be slow if we need to split a lot of coins, such as 1 million.
2. Error handling is poor right now.

#### Local Initializer
This is what we could use when testing locally. It simply injects a bunch of gas coins into the storage layer, without actually owning them in any network.

### RPC Server
An HTTP server is implemented to take the following 3 requests:
- GET("/"): Checks the health of the server
- POST("/v1/reserve_gas"): Takes a [`ReserveGasRequest`](src/rpc/rpc_types.rs) parameter in JSON form, and returns [`ReserveGasResponse`](src/rpc/rpc_types.rs). The response contains the transaction data and the sponsor signature.
- POST("/v1/release_gas"): Takes a [`ReleaseGasRequest`](src/rpc/rpc_types.rs) parameter in JSON form.

### CLI
The `sui-gas-staiton` binary currently supports 3 commands:
1. `init`: This invokes the Production Initializer and initialize the global gas pool using real network data.
2. `init-for-local-testing-only`: This invokes the Local Initializer and initialize the global gas pool using fake data.
3. `start`: This starts a gas station instance that contains the RPC server, Gas Station Core, and connection to the Storage layer.
