// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use moka::sync::SegmentedCache;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use sui_types::base_types::ObjectID;
use sui_types::object::Owner;
use sui_types::transaction::{InputObjectKind, TransactionData, TransactionDataAPI};
use tracing::{debug, warn};

const CACHE_SIZE: u64 = 1000000;

#[async_trait::async_trait]
pub trait MultiGetObjectOwners: Send + Sync + 'static {
    async fn multi_get_object_owners(
        &self,
        object_ids: Vec<ObjectID>,
    ) -> Result<HashMap<ObjectID, (Owner, u64)>, anyhow::Error>;
}

/// A component that manages object locks for actively executing transactions.
/// This can help avoid equivocation of transactions that concurrently modify
/// the same address-owned objects.
pub struct ObjectLockManager {
    /// A cache that tracks whether an object is fastpath address-owned and its version.
    /// We store (version, is_address_owned) where version is the last seen version of the object
    /// and is_address_owned is true if the object is address-owned, false if it is not.
    /// Using SegmentedCache for better concurrent performance.
    address_owned_cache: SegmentedCache<ObjectID, (u64, bool)>,
    /// Tracks the objects that are currently locked due to
    /// active execution of a transaction.
    locked_owned_objects: Arc<RwLock<HashSet<ObjectID>>>,
    sui_client: Arc<dyn MultiGetObjectOwners>,
}

/// A RAII guard that manages object locks for a transaction.
///
/// This struct is returned by `ObjectLockManager::try_acquire_locks`.
/// It will automatically release the locks when the guard is dropped.
pub struct ObjectLocks {
    reservation_id: u64,
    locked_objects: Vec<ObjectID>,
    global_locked_owned_objects: Arc<RwLock<HashSet<ObjectID>>>,
}

impl ObjectLocks {
    fn remove_locks_from_set(&self, locks: &mut HashSet<ObjectID>) {
        for obj in &self.locked_objects {
            locks.remove(obj);
        }
        debug!(
            ?self.reservation_id,
            "Removed locks for objects: {:?}",
            self.locked_objects
        );
    }
}

impl Drop for ObjectLocks {
    fn drop(&mut self) {
        match self.global_locked_owned_objects.write() {
            Ok(mut locks) => {
                self.remove_locks_from_set(&mut locks);
            }
            Err(poisoned) => {
                let mut locks = poisoned.into_inner();
                warn!(?self.reservation_id, "Poisoned RwLock for objects: {:?}", self.locked_objects);
                self.remove_locks_from_set(&mut locks);
            }
        }
    }
}

impl ObjectLockManager {
    pub fn new(sui_client: Arc<dyn MultiGetObjectOwners>) -> Self {
        Self {
            address_owned_cache: SegmentedCache::new(CACHE_SIZE, 8),
            locked_owned_objects: Arc::new(RwLock::new(HashSet::new())),
            sui_client,
        }
    }

    /// Acquires locks for all the owned objects (except gas) in the transaction.
    ///
    /// This function will return an error if the locks cannot be acquired
    /// due to equivocation. We do not need to wait or retry because the other
    /// transaction will mutate the locked object and advance its version.
    /// This transaction will never succeed to execute.
    pub async fn try_acquire_locks(
        &self,
        reservation_id: u64,
        tx_data: &TransactionData,
    ) -> Result<ObjectLocks, anyhow::Error> {
        debug!(?reservation_id, "Trying to acquire object locks");
        let imm_or_owned_objects = self.get_imm_or_owned_non_gas_objects(tx_data)?;
        {
            // While some of the objects in imm_or_owned_objects may be immutable,
            // we could still perform a preliminary check to see if any object is already locked.
            // This can help avoid unnecessary write locks, as well as unnecessary queries to the
            // RPC nodes when trying to filtering out owned objects.
            let Ok(locks) = self.locked_owned_objects.read() else {
                anyhow::bail!("Failed to acquire read lock for locked_owned_objects");
            };
            for (obj, _) in &imm_or_owned_objects {
                if locks.contains(obj) {
                    debug!(?reservation_id, "Object is already locked: {:?}", obj);
                    anyhow::bail!("Object is already locked: {:?}", obj);
                }
            }
            // locks is dropped here.
        }
        let owned_objects = self.filter_owned_objects(imm_or_owned_objects).await?;
        let Ok(mut locks) = self.locked_owned_objects.write() else {
            anyhow::bail!("Failed to acquire write lock for locked_owned_objects");
        };
        for obj in &owned_objects {
            if locks.contains(obj) {
                debug!(?reservation_id, "Object is already locked: {:?}", obj);
                anyhow::bail!("Object is already locked: {:?}", obj);
            }
        }
        locks.extend(&owned_objects);
        Ok(ObjectLocks {
            reservation_id,
            locked_objects: owned_objects,
            global_locked_owned_objects: self.locked_owned_objects.clone(),
        })
    }

    fn get_imm_or_owned_non_gas_objects(
        &self,
        tx_data: &TransactionData,
    ) -> Result<Vec<(ObjectID, u64)>, anyhow::Error> {
        let gas_object_ids: HashSet<_> =
            tx_data.gas_data().payment.iter().map(|obj| obj.0).collect();
        Ok(tx_data
            .input_objects()?
            .into_iter()
            .filter_map(|obj| match obj {
                InputObjectKind::ImmOrOwnedMoveObject(obj_ref) => {
                    // Filter out gas objects because they are provided by the gas pool,
                    // and they will never equivocate. So we do not need to lock them.
                    if gas_object_ids.contains(&obj_ref.0) {
                        None
                    } else {
                        Some((obj_ref.0, obj_ref.1.value()))
                    }
                }
                _ => None,
            })
            .collect())
    }

    /// Given a list of object IDs that are known to be either address-owned or immutable,
    /// return the list of address-owned objects.
    ///
    /// This function will query the owner of the objects from the cache.
    /// If the mutability of an object is not in the cache, it will make a batch query to the Sui client to get their mutability,
    /// and add to the cache.
    ///
    /// The returned list of objects are owned by an address.
    async fn filter_owned_objects(
        &self,
        objects: Vec<(ObjectID, u64)>, // (id, version) pairs
    ) -> Result<Vec<ObjectID>, anyhow::Error> {
        let mut owned_objects = Vec::new();
        let mut objects_to_query = HashMap::new();

        for (obj_id, version) in objects {
            match self.address_owned_cache.get(&obj_id) {
                Some((cached_version, is_address_owned)) => {
                    match version.cmp(&cached_version) {
                        std::cmp::Ordering::Less => {
                            anyhow::bail!(
                                "Object version is out of date: {:?}, provided: {}, latest: {}",
                                obj_id,
                                version,
                                cached_version
                            );
                        }
                        std::cmp::Ordering::Equal => {
                            // If versions match, use the cached value.
                            if is_address_owned {
                                owned_objects.push(obj_id);
                            }
                        }
                        std::cmp::Ordering::Greater => {
                            // If provided version is newer, we need to re-query
                            objects_to_query.insert(obj_id, version);
                        }
                    }
                }
                None => {
                    objects_to_query.insert(obj_id, version);
                }
            }
        }

        if !objects_to_query.is_empty() {
            let ids = objects_to_query.keys().cloned().collect();
            let fetched_objects = self.sui_client.multi_get_object_owners(ids).await?;

            for (obj, (owner, fetched_version)) in fetched_objects {
                let input_version = objects_to_query[&obj];
                if fetched_version > input_version {
                    anyhow::bail!(
                        "Object version is out of date: {:?}, provided: {}, latest: {}",
                        obj,
                        input_version,
                        fetched_version,
                    );
                }
                // It is technically possible that the fetched_version is out of date.
                // This is rare and we do not handle it here.
                // In the worst case, we cache an older version and assumes the ownership
                // hasn't changed since then. It will be updated when the next transaction
                // uses the object.
                let is_address_owned = matches!(owner, Owner::AddressOwner(_));
                self.address_owned_cache
                    .insert(obj, (fetched_version, is_address_owned));
                if is_address_owned {
                    owned_objects.push(obj);
                }
            }
        }
        Ok(owned_objects)
    }

    /// After we have executed a transaction, we can update the cache using the effects.
    /// This allows us to update the latest version of objects that are mutated.
    /// This is important since we rely on version to determine if an object is address-owned
    /// from the cache.
    pub fn update_cache_post_execution(
        &self,
        tx_data: &TransactionData,
        // This is obtained through effects.mutated() in the caller.
        effects_mutated_objects: Vec<(ObjectID, Owner, u64)>,
    ) {
        let imm_or_owned_input_objects: HashSet<_> = self
            .get_imm_or_owned_non_gas_objects(tx_data)
            // unwrap safe since we should have already checked the validity of the transaction
            // prior to execution.
            .unwrap()
            .into_iter()
            .map(|(id, _)| id) // Only need IDs for the set
            .collect();
        for (obj, owner, version) in effects_mutated_objects {
            if imm_or_owned_input_objects.contains(&obj) {
                self.address_owned_cache
                    .insert(obj, (version, matches!(owner, Owner::AddressOwner(_))));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use sui_types::base_types::random_object_ref;
    use sui_types::base_types::{SequenceNumber, SuiAddress};
    use sui_types::digests::ObjectDigest;
    use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
    use sui_types::transaction::TransactionKind;
    use sui_types::transaction::{CallArg, ObjectArg};

    struct MockSuiClient {
        objects: Arc<Mutex<HashMap<ObjectID, (Owner, u64)>>>,
    }

    impl MockSuiClient {
        fn new_empty() -> Arc<Self> {
            Arc::new(Self {
                objects: Arc::new(Mutex::new(HashMap::new())),
            })
        }

        fn new_with_owners(objects: HashMap<ObjectID, (Owner, u64)>) -> Arc<Self> {
            Arc::new(Self {
                objects: Arc::new(Mutex::new(objects)),
            })
        }

        fn update_owner(&self, id: ObjectID, owner: Owner, version: u64) {
            let mut objects = self.objects.lock().unwrap();
            objects.insert(id, (owner, version));
        }
    }

    #[async_trait::async_trait]
    impl MultiGetObjectOwners for MockSuiClient {
        async fn multi_get_object_owners(
            &self,
            object_ids: Vec<ObjectID>,
        ) -> Result<HashMap<ObjectID, (Owner, u64)>, anyhow::Error> {
            let objects = self.objects.lock().unwrap();
            Ok(object_ids
                .into_iter()
                .filter_map(|id| {
                    objects
                        .get(&id)
                        .map(|(owner, version)| (id, (owner.clone(), *version)))
                })
                .collect())
        }
    }

    fn create_test_tx_data(
        address_owned: Vec<(ObjectID, u64)>, // Now takes (id, version) pairs
        immutable: Vec<(ObjectID, u64)>,
        shared: Vec<ObjectID>,
    ) -> TransactionData {
        let dummy_address = SuiAddress::random_for_testing_only();
        let mut ptb = ProgrammableTransactionBuilder::new();
        for (obj, version) in address_owned.into_iter().chain(immutable) {
            ptb.input(CallArg::Object(ObjectArg::ImmOrOwnedObject((
                obj,
                SequenceNumber::from_u64(version),
                ObjectDigest::random(),
            ))))
            .unwrap();
        }
        for obj in shared {
            ptb.input(CallArg::Object(ObjectArg::SharedObject {
                id: obj,
                initial_shared_version: SequenceNumber::new(),
                mutable: true,
            }))
            .unwrap();
        }

        TransactionData::new(
            TransactionKind::ProgrammableTransaction(ptb.finish()),
            dummy_address,
            random_object_ref(),
            1,
            1,
        )
    }

    #[tokio::test]
    async fn test_empty_lock_acquisition() {
        let client = MockSuiClient::new_empty();
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![], vec![], vec![]);

        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        // Show that it won't lock any objects, including the gas.
        assert!(locks.locked_objects.is_empty());
        drop(locks);
    }

    #[tokio::test]
    async fn test_basic_lock_and_release() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);

        // Acquire locks
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();

        assert_eq!(locks.locked_objects.len(), 1);
        assert_eq!(locks.locked_objects[0], obj_id);

        // Verify lock is held
        {
            let locked_objects = manager.locked_owned_objects.read().unwrap();
            assert!(locked_objects.contains(&obj_id));
        }

        // Drop locks
        drop(locks);

        // Verify lock is released
        {
            let locked_objects = manager.locked_owned_objects.read().unwrap();
            assert!(!locked_objects.contains(&obj_id));
        }
    }

    #[tokio::test]
    async fn test_concurrent_lock_acquisition() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);

        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);

        assert!(manager.try_acquire_locks(2, &tx_data).await.is_err());
        drop(locks);

        let locks = manager.try_acquire_locks(2, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);
    }

    #[tokio::test]
    async fn test_version_based_cache_behavior() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client.clone()));

        // First transaction with version 1
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Update cache with version 2
        manager.update_cache_post_execution(
            &tx_data,
            vec![(
                obj_id,
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                2,
            )],
        );

        // Try with version 1 - should fail since version is out of date
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);
        assert!(manager.try_acquire_locks(2, &tx_data).await.is_err());

        // Try with version 2 - should use cache and succeed
        let tx_data = create_test_tx_data(vec![(obj_id, 2)], vec![], vec![]);
        let locks = manager.try_acquire_locks(3, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Update object to version 3
        client.update_owner(
            obj_id,
            Owner::AddressOwner(SuiAddress::random_for_testing_only()),
            3,
        );

        // Try with version 3 - should re-query and succeed
        let tx_data = create_test_tx_data(vec![(obj_id, 3)], vec![], vec![]);
        let locks = manager.try_acquire_locks(4, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
    }

    #[tokio::test]
    async fn test_lock_acquisition_version_mismatch() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));

        // Try to acquire locks with version 2, but the object is only version 1.
        // We allow this to happenand it will still be locked.
        let tx_data = create_test_tx_data(vec![(obj_id, 2)], vec![], vec![]);
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Acquiring locks at version 1 should also succeed because it's what we cached.
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);
        let locks = manager.try_acquire_locks(2, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Acquiring locks at version 0 should fail because it's out of date.
        let tx_data = create_test_tx_data(vec![(obj_id, 0)], vec![], vec![]);
        assert!(manager.try_acquire_locks(3, &tx_data).await.is_err());
    }

    #[tokio::test]
    async fn test_cache_update_when_object_frozen() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client.clone()));
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);

        // First acquire locks - should succeed since object is address owned
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Simulate transaction execution where object becomes immutable at version 2
        let mutated_objects = vec![(obj_id, Owner::Immutable, 2)];
        manager.update_cache_post_execution(&tx_data, mutated_objects);

        // Try with version 1 - should fail since version is out of date
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);
        assert!(manager.try_acquire_locks(2, &tx_data).await.is_err());

        // Try with version 2 - should not lock the object since it's now immutable
        let tx_data = create_test_tx_data(vec![(obj_id, 2)], vec![], vec![]);
        let locks = manager.try_acquire_locks(3, &tx_data).await.unwrap();
        assert!(locks.locked_objects.is_empty());
    }

    #[tokio::test]
    async fn test_cache_update_shared_then_owned() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            (
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
                1,
            ),
        );

        let client = MockSuiClient::new_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client.clone()));
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);

        // First acquire locks - should succeed since object is address owned
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
        drop(locks);

        // Update object to shared at version 2
        let shared_owner = Owner::Shared {
            initial_shared_version: SequenceNumber::new(),
        };

        // Simulate transaction execution where object becomes shared at version 2
        manager.update_cache_post_execution(&tx_data, vec![(obj_id, shared_owner, 2)]);

        // Try with version 1 - should fail since version is out of date
        let tx_data = create_test_tx_data(vec![(obj_id, 1)], vec![], vec![]);
        assert!(manager.try_acquire_locks(2, &tx_data).await.is_err());

        // Try with version 2 - should not lock shared object
        let tx_data = create_test_tx_data(vec![(obj_id, 2)], vec![], vec![]);
        let locks = manager.try_acquire_locks(3, &tx_data).await.unwrap();
        assert!(locks.locked_objects.is_empty());
        drop(locks);

        // Update object to address owned at version 3
        let new_address_owner = Owner::AddressOwner(SuiAddress::random_for_testing_only());
        client.update_owner(obj_id, new_address_owner.clone(), 3);

        // Try with version 3 - should re-query lock the object since it's address owned again
        let tx_data = create_test_tx_data(vec![(obj_id, 3)], vec![], vec![]);
        let locks = manager.try_acquire_locks(4, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 1);
    }
}
