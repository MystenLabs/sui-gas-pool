# Sui Gas Pool

Sui Gas Pool is a service that powers sponsored transactions on Sui at scale. It manages a database of gas coins owned
by a sponsor address and provides APIs to reserve gas coins and use them to pay for transactions. It achieves
scalability and high throughput by managing a large number of gas coin objects in the pool, so that it can sponsor a
large number of transactions concurrently.

## User Flow

A typical flow that interacts with the gas pool service looks like the following:

1. App or client prepares a transaction without gas payment, sends it to an internal server.
2. The internal server talks to the gas pool service to reserve gas coins for the given budget specified by the
   transaction.
3. The gas pool reserves gas coins and returns them to the internal server.
4. The internal server sends the complete transaction back to the app/client.
5. The app/client asks the user to sign the complete transaction, and sends back the sender signed transaction to the
   internal server.
6. The internal server then sends the sender signed transaction to the gas pool service to execute the transaction.
7. The gas pool service executes the transaction though a fullnode and returns the effects to the internal server, which
   then forwards it back to the app/client. The used gas coins are also freed up and ready for reservation again.

## Architecture

A gas pool service instance corresponds to one sponsor address. A gas pool service instance will contain the following
components:

1. Redis Storage. There should be a single Redis instance per gas pool service. This stores all the data that need to be
   persisted, including the gas coin objects in the pool, reservation information and etc.
2. One or more gas pool servers. This is the binary built from this repository, and it is ok to deploy multiple
   instances of them, as long as they all connect to the same Redis server. Each of these servers provide RPC interfaces
   to serve requests.
3. (Optional) KMS Sidecar. There should be a single KMS sidecar instance per gas pool service. The KMS defines the
   sponsor address and can sign transactions securely. Optionally we can also store the private key in memory without
   using a KMS sidecar.

## Redis Storage

The storage layer stores all the gas coins in the pool and reservation information. It is the only place where we
persist data.
It uses Redis store as the backend, and Lua scripts to control the logic.
Detailed documentation of each Lua script can be found
in [link](https://www.notion.so/mystenlabs/src/storage/redis/lua_scripts/).

## Gas Pool Server

The Gas Pool Server contains a RPC server that accepts JSON-RPC requests, as well as an initializer that is able to
initialize and fund gas coins to the pool:

1. Upon requesting gas coins, it's able to obtain gas coins from the storage layer and return them to the caller.
2. Caller can follow up with a transaction execution request that uses previously reserved coins, and the gas station
   core will drive the execution of the transaction, automatically release the coins back to the storage layer after the
   transaction is executed.
3. It's able to automatically release reserved gas coins back to the storage after the requested duration expires.

### RPC Server

The gas pool service starts a RPC Server that listens on a specified port. It supports permission control through barer
secret token. An internal server that communicates with the gas pool service must specify the token in the request. This
is also why an internal server is needed such that the barer token is not exposed to the public.
An HTTP server is implemented to take the following 3 requests:

- GET("/"): Checks the health of the server
- POST("/v1/reserve_gas"): Takes a [`ReserveGasRequest`](https://www.notion.so/mystenlabs/src/rpc/rpc_types.rs)
  parameter in JSON form, and
  returns [`ReserveGasResponse`](https://www.notion.so/mystenlabs/src/rpc/rpc_types.rs).
- POST("/v1/execute_tx"): Takes a [`ExecuteTxRequest`](https://www.notion.so/mystenlabs/src/rpc/rpc_types.rs) parameter
  in JSON form, and
  returns [`ExecuteTxResponse`](https://www.notion.so/mystenlabs/src/rpc/rpc_types.rs).

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

### Gas Pool Initializer

The Gas Pool Initializer is able to initialize the global gas pool, as well as processing new funds and adding new coins
to the gas pool.
When we are starting up the gas pool for a given sponsor address for the first time, it will trigger the initialization
process. It looks at all the SUI coins currently owned by the sponsor address, and split them into gas coins with a
specified target balance. Once a day, it also looks at whether there is any coin owned by the sponsor address with a
very large balance (NEW_COIN_BALANCE_FACTOR_THRESHOLD \* target_init_balance), and if so it triggers initialization
process again on the newly detected coin. This allows us add funding to the gas pool.
To speed up the initialization time, it is able to split coins into smaller coins in parallel.
Before each initialization run, it acquires a lock from the store to ensure that no other initialization task is running
at the same time. The lock expires automatically after 12 hours.
This allows us to run multiple gas servers for the same sponsor address.

### Transaction Signing

The sponsor will need to sign transactions since it owns the gas coins. The gas pool supports two different signer
implementations:

1. KMS Sidecar: This allows us to manage private keys in a secure key management service such as AWS KMS. You will need
   to run a KMS sidecar that accepts signing requests through a JSON-RPC endpoint, that talks to AWS and signs
   transactions. We provided a [sample implementation](sample_kms_sidecar/) of such sidecar in the repo.
2. In-memory: This allows the gas pool server to load a SuiKeyPair directly from file and use it to sign transactions.

## Binaries

### `sui-gas-station` Binary

The binary takes a an argument:

- `--config-path` (required): Path to the config file.

### `tool` Binary

The `tool` binary currently supports a few helper commands:

1. `benchmark`: This starts a stress benchmark that continuously send gas reservation request to the gas station server,
   and measures number of requests processed per second. Each reservation expires automatically after 1 second so the
   unused gas are put back to the pool.
2. `generate-sample-config`: This generates a sample config file that can be used to start the gas station server.
3. `cli`: Provides a few CLI commands to interact with the gas station server.

## Deployment

Below describes the steps to deploy a gas pool service:

1. Get a sponsor address keypair, either by generating it manually if you want to use in-memory signer, or get a KMS
   instance from some cloud providers (or implement your own).
2. Send a sufficiently funded SUI coin into that address. This will be the initial funding of the gas pool.
3. Deploy a Redis instance.
4. Create a YAML config file (see details below).
5. Pick a secure secret token for the RPC server, this will be passed through the `GAS_STATION_AUTH` environment
   variable when starting the gas pool server.
6. Deploy the gas pool server.

To create a YAML config file, you can use the following command to generate a sample config:

`tool generate-sample-config --config-path sample.yaml --with-sidecar-signer`

```yaml
---
signer-config:
  sidecar:
    sidecar_url: "http://localhost:3000"
rpc-host-ip: 0.0.0.0
rpc-port: 9527
metrics-port: 9184
gas-pool-config:
  redis:
    redis_url: "redis:://127.0.0.1"
fullnode-url: "http://localhost:9000"
coin-init-config:
  target-init-balance: 100000000
  refresh-interval-sec: 86400
daily-gas-usage-cap: 1500000000000
```

If you want to use in-memory signer, you can remove `--with-sidecar-signer` from the command.

A description of these fields:

- sidecar_url: This is the RPC endpoint of the KMS sidecar.
- rpc-host-ip: The IP of the gas pool RPC server, usually just 0.0.0.0.
- rpc-port: The port that RPC server runs on.
- metrics-port: The port where some metric service could go and grab metrics and logging.
- redis_url: The full URL of the Redis instance.
- fullnode-url: The fullnode that the gas pool will be talking to.
- coin-init-config
  - target-init-balance: The targeting initial balance of each coin (in MIST). For instance if you specify 100000000
    which is 0.1 SUI, the gas pool will attempt to split its gas coin into smaller gas coins each with 0.1 SUI balance
    during initialization.
  - refresh-interval-sec: The interval to look at all gas coins owned by the sponsor again and see if some new funding
    has been added.
- daily-gas-usage-cap: The total amount of gas usage allowed per day, as a safety cap.
