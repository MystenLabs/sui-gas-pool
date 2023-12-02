# Enoki Gas Pool
This is a high-level summary of how Sui Gas Station works.

## Assumptions
The Gas Station can only be reached from an internal server, which handles rate limiting and authentication. The Gas Station blindly trusts the requests it receives.

## Architecture
There are 5 components in the Gas Station:
### Storage
The storage layer stores the global gas pool information.
Any implementation must support the following two API functions:
- `fn reserve_gas_coins(&self, sponsor_address: SuiAddress, target_budget: u64) -> Vec<GasCoin>;`
- `fn update_gas_coins(&self, sponsor_address: SuiAddress, released_gas_coins: Vec<GasCoin>, deleted_gas_coins: Vec<ObjectID>);
Where `GasCoin` is defined as:
```rust
pub struct GasCoin {
    pub object_ref: ObjectRef,
    pub balance: u64,
}
```
`reserve_gas_coins` is able to take off potentially more than 1 gas coins from an internal queue atomically until the total accumulated balance reaches `target_budget`, or give up if it cannot find enough coins to meet the requirement even after searching for N coins (N can be implementation-specific, but must be less than 256 since it's our gas payment size cap).
This also takes advantage of the fact that gas smashing is able to merge gas coins during transaction execution. So if we have coins that have low balance, by returning multiple of them we are doing automatic dust collection.
It supports multiple sponsors, and each sponsor has its own queue of gas coins.
Caller must specify which sponsor address to use.

The storage layer is expected to keep track the list of coins that are currently reserved. This is important because although rare, the gas station server may crash after reserving a coin and never come back up. This means over time some coins will be reserved but never released. If we have a table in the storage that keeps track of reserved coins along with their reservation time, we can then run a GC process from time to time to recollect gas coins that were reserved a long time ago.

`update_gas_coins` is able to add back gas coins to the gas pool, or mark certain gas coin permanently deleted. It can be called either when we are returning gas coins after using them, or adding new coins to the pool.
Note that the released gas coins are the critical part to ensure protocol correctness, while the deleted_gas_coins is used to remove coins from the currently reserved coin list, so that we don't have to keep track of them forever.

### Gas Station Core
The Gas Station Core implements the core gas station logic that is able to process RPC requests and communicate with the Storage layer.
It has the following features:
1. Upon requesting gas coins, it's able to obtain gas coins from the storage layer, remember it in memory, and return them to the caller.
2. It's able to automatically release reserved gas coins back to the storage after the requested duration expires.
3. Caller can then follow up with a transaction execution request that uses a previously reserved coin list, and the gas station core will drive the execution of the transaction, automatically release the coins back to the storage layer after the transaction is executed.

### Gas Pool Initializer
A Gas Pool Initializer is able to initialize the global gas pool.
When we are setting up the gas pool, we will need to run the initialization exactly once. It is able to look at all the SUI coins currently owned by the sponsor address, and split them into gas coins with a specified target balance.
This is done by splitting coins into smaller coins in parallel to minimize the amount of time to split all.

### RPC Server
An HTTP server is implemented to take the following 3 requests:
- GET("/"): Checks the health of the server
- POST("/v1/reserve_gas"): Takes a [`ReserveGasRequest`](src/rpc/rpc_types.rs) parameter in JSON form, and returns [`ReserveGasResponse`](src/rpc/rpc_types.rs).
- POST("/v1/execute_tx"): Takes a [`ExecuteTxRequest`](src/rpc/rpc_types.rs) parameter in JSON form, and returns [`ExecuteTxResponse`](src/rpc/rpc_types.rs).

```rust
pub struct ReserveGasRequest {
    pub gas_budget: u64,
    /// If request_sponsor is None, the station will pick one automatically.
    pub request_sponsor: Option<SuiAddress>,
    pub reserve_duration_secs: u64,
}

pub struct ReserveGasResponse {
    pub gas_coins: Option<(SuiAddress, Vec<ObjectRef>)>,
    pub error: Option<String>,
}

pub struct ExecuteTxRequest {
    pub tx: TransactionData,
    pub user_sig: GenericSignature,
}

pub struct ExecuteTxResponse {
    pub effects: Option<SuiTransactionBlockEffects>,
    pub error: Option<String>,
}
```

### CLI
The `sui-gas-staiton` binary currently supports 3 commands:
1. `init`: This invokes the Production Initializer and initialize the global gas pool using real network data.
2. `start`: This starts a gas station instance that contains the RPC server, Gas Station Core, and connection to the Storage layer.
3. `benchmark`: This starts a stress benchmark that continuously send gas reservation request to the gas station server, and measures number of requests processed per second. Each reservation expires automatically after 1 second so the unused gas are put back to the pool.


## TODOs
1. Add metrics
2. Add more logging
3. Add more tests
4. Integrate with Redis DB
5. Integrate with AWS KMS
6. Add crash recovery using local database.

