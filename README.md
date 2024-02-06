# Enoki Gas Pool
This is a high-level summary of how Sui Gas Station works.

## Assumptions
The Gas Station can only be reached from an internal server, which handles rate limiting and authentication. The Gas Station blindly trusts the requests it receives.

## Architecture
There are 5 components in the Gas Station:
### Storage
The storage layer stores the global gas pool information.
It uses Redis store as the backend, and Lua scripts to control the logic.

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
    /// Desired gas budget. The response will contain gas coins that have total balance >= gas_budget.
    pub gas_budget: u64,
    /// The reserved gas coins will be released back to the pool after this duration expires.
    pub reserve_duration_secs: u64,
}

pub struct ReserveGasResponse {
    pub result: Option<ReserveGasResult>,
    pub error: Option<String>,
}

pub struct ReserveGasResult {
    pub sponsor_address: SuiAddress,
    pub reservation_id: ReservationID,
    pub gas_coins: Vec<SuiObjectRef>,
}

pub struct ExecuteTxRequest {
    /// This must be the same reservation ID returned in ReserveGasResponse.
    pub reservation_id: ReservationID,
    /// BCS serialized transaction data bytes without its type tag, as base-64 encoded string.
    pub tx_bytes: Base64,
    /// User signature (`flag || signature || pubkey` bytes, as base-64 encoded string). Signature is committed to the intent message of the transaction data, as base-64 encoded string.
    pub user_sig: Base64,
}

pub struct ExecuteTxResponse {
    pub effects: Option<SuiTransactionBlockEffects>,
    pub error: Option<String>,
}
```

### `sui-gas-staiton` Binary
The binary takes a few command line arguments:
- `--config-path` (required): Path to the config file.
- `--force-init-gas-pool` (optional): If specified, the gas pool will be initialized with the specified target balance from the config. This should only be called once per address, and may take a while.

### `tool` Binary
The `tool` binary currently supports a few helper commands:
1. `benchmark`: This starts a stress benchmark that continuously send gas reservation request to the gas station server, and measures number of requests processed per second. Each reservation expires automatically after 1 second so the unused gas are put back to the pool.
2. `generate-sample-config`: This generates a sample config file that can be used to start the gas station server.
3. `cli`: Provides a few CLI commands to interact with the gas station server.

### Deployment
Step 1:
Put up a config file.
First of all one can run
```bash
sui-gas-station generate-sample-config --config-path config.yaml
```
to generate a sample config file.
Then one can edit the config file to fill in the fields.
The keypair field is the serialized SuiKeyPair that can be found in a typical .keystore file.

Step 2:
Start a Redis server.

Step 3:
Start the gas station.
```bash
GAS_STATION_AUTH=<secret> sui-gas-station --config-path config.yaml
```
If this is the first time we are running the gas station, we will need to initialize the gas pool first.
```bash
GAS_STATION_AUTH=<secret> sui-gas-station --config-path config.yaml --force-init-gas-pool
```
