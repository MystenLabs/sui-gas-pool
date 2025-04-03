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
    ) -> Result<HashMap<ObjectID, Owner>, anyhow::Error>;
}

/// A component that manages object locks for actively executing transactions.
/// This can help avoid equivocation of transactions that concurrently modify
/// the same address-owned objects.
pub struct ObjectLockManager {
    /// A cache that tracks whether an object is address-owned.
    /// Using SegmentedCache for better concurrent performance.
    address_owned_cache: SegmentedCache<ObjectID, bool>,
    /// A cache that tracks the objects that are currently locked due to
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
        let imm_or_owned_objects = self.get_imm_or_owned_objects(tx_data)?;
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

    fn get_imm_or_owned_objects(
        &self,
        tx_data: &TransactionData,
    ) -> Result<Vec<ObjectID>, anyhow::Error> {
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
                        Some(obj_ref.0)
                    }
                }
                _ => None,
            })
            .collect())
    }

    /// Given a list of object IDs, return the list of address-owned objects, which are mutable.
    ///
    /// This function will query the owner of the objects from the cache.
    /// If the mutability of an object is not in the cache, it will make a batch query to the Sui client to get their mutability,
    /// and add to the cache.
    ///
    /// The returned list of objects are owned by an address.
    async fn filter_owned_objects(
        &self,
        objects: Vec<ObjectID>,
    ) -> Result<Vec<ObjectID>, anyhow::Error> {
        let mut mutable_objects = Vec::new();
        let mut objects_to_query = Vec::new();

        for obj in objects {
            match self.address_owned_cache.get(&obj) {
                Some(true) => mutable_objects.push(obj),
                Some(false) => {} // Ignore known immutable objects
                None => objects_to_query.push(obj),
            }
        }

        if !objects_to_query.is_empty() {
            let mutability_results = self
                .sui_client
                .multi_get_object_owners(objects_to_query)
                .await?;

            for (obj, owner) in mutability_results {
                let is_address_owned = matches!(owner, Owner::AddressOwner(_));
                self.address_owned_cache.insert(obj, is_address_owned);
                if is_address_owned {
                    mutable_objects.push(obj);
                }
            }
        }
        Ok(mutable_objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use sui_types::base_types::random_object_ref;
    use sui_types::base_types::{SequenceNumber, SuiAddress};
    use sui_types::digests::ObjectDigest;
    use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;
    use sui_types::transaction::TransactionKind;
    use sui_types::transaction::{CallArg, ObjectArg};

    struct MockSuiClient {
        owners: HashMap<ObjectID, Owner>,
    }

    #[async_trait::async_trait]
    impl MultiGetObjectOwners for MockSuiClient {
        async fn multi_get_object_owners(
            &self,
            object_ids: Vec<ObjectID>,
        ) -> Result<HashMap<ObjectID, Owner>, anyhow::Error> {
            Ok(object_ids
                .into_iter()
                .filter_map(|id| self.owners.get(&id).map(|owner| (id, owner.clone())))
                .collect())
        }
    }

    fn create_mock_client() -> Arc<MockSuiClient> {
        Arc::new(MockSuiClient {
            owners: HashMap::new(),
        })
    }

    fn create_mock_client_with_owners(owners: HashMap<ObjectID, Owner>) -> Arc<MockSuiClient> {
        Arc::new(MockSuiClient { owners })
    }

    fn create_test_tx_data(
        address_owned: Vec<ObjectID>,
        immutable: Vec<ObjectID>,
        shared: Vec<ObjectID>,
    ) -> TransactionData {
        let dummy_address = SuiAddress::random_for_testing_only();
        let mut ptb = ProgrammableTransactionBuilder::new();
        for obj in address_owned.into_iter().chain(immutable) {
            ptb.input(CallArg::Object(ObjectArg::ImmOrOwnedObject((
                obj,
                SequenceNumber::new(),
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
    async fn test_basic_lock_acquisition() {
        let client = create_mock_client();
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![], vec![], vec![]);

        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        // Show that it won't lock any objects, including the gas.
        assert!(locks.locked_objects.is_empty());
    }

    #[tokio::test]
    async fn test_lock_and_release() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            Owner::AddressOwner(SuiAddress::random_for_testing_only()),
        );

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![obj_id], vec![], vec![]);

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
            Owner::AddressOwner(SuiAddress::random_for_testing_only()),
        );

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![obj_id], vec![], vec![]);

        // Try to acquire locks concurrently
        let manager_clone = manager.clone();
        let tx_data_clone = tx_data.clone();
        let handle1 = manager.try_acquire_locks(1, &tx_data);
        let handle2 = manager_clone.try_acquire_locks(2, &tx_data_clone);

        let (locks1, locks2) = tokio::join!(handle1, handle2);
        let locks1 = locks1.unwrap();
        let locks2 = locks2;

        // Second acquisition should fail
        assert!(locks2.is_err());

        // Release first lock
        drop(locks1);

        // Now second acquisition should succeed
        let manager_clone = manager.clone();
        let locks2 = manager_clone.try_acquire_locks(2, &tx_data).await.unwrap();
        assert_eq!(locks2.locked_objects[0], obj_id);
    }

    #[tokio::test]
    async fn test_cache_behavior() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            Owner::AddressOwner(SuiAddress::random_for_testing_only()),
        );

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![obj_id], vec![], vec![]);

        // First call should query the client
        let manager_clone = manager.clone();
        let tx_data_clone = tx_data.clone();
        let locks1 = manager_clone
            .try_acquire_locks(1, &tx_data_clone)
            .await
            .unwrap();
        drop(locks1);

        // Second call should use cache
        let locks2 = manager.try_acquire_locks(2, &tx_data).await.unwrap();
        assert_eq!(locks2.locked_objects[0], obj_id);
    }

    #[tokio::test]
    async fn test_immutable_objects() {
        let immutable_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(immutable_id, Owner::Immutable);

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![], vec![immutable_id], vec![]);

        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert!(locks.locked_objects.is_empty());
    }

    #[tokio::test]
    async fn test_lock_timeout() {
        let obj_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            obj_id,
            Owner::AddressOwner(SuiAddress::random_for_testing_only()),
        );

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![obj_id], vec![], vec![]);

        // Acquire first lock
        let manager_clone = manager.clone();
        let tx_data_clone = tx_data.clone();
        let locks1 = manager_clone
            .try_acquire_locks(1, &tx_data_clone)
            .await
            .unwrap();

        // Try to acquire same lock - should timeout
        let start = Instant::now();
        let result = manager.try_acquire_locks(2, &tx_data).await;

        assert!(result.is_err());
        assert!(start.elapsed() >= LOCK_ACQUISITION_TIMEOUT);

        // Release first lock
        drop(locks1);
    }

    #[tokio::test]
    async fn test_multiple_objects() {
        let obj_ids: Vec<_> = (0..3).map(|_| ObjectID::random()).collect();
        let mut owners = HashMap::new();
        for obj_id in &obj_ids {
            owners.insert(
                *obj_id,
                Owner::AddressOwner(SuiAddress::random_for_testing_only()),
            );
        }

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(obj_ids.clone(), vec![], vec![]);

        // Should acquire all locks
        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert_eq!(locks.locked_objects.len(), 3);

        // Verify all objects are locked
        {
            let locked_objects = manager.locked_owned_objects.read().unwrap();
            for obj_id in obj_ids {
                assert!(locked_objects.contains(&obj_id));
            }
        }
    }

    #[tokio::test]
    async fn test_shared_objects() {
        let shared_id = ObjectID::random();
        let mut owners = HashMap::new();
        owners.insert(
            shared_id,
            Owner::Shared {
                initial_shared_version: SequenceNumber::new(),
            },
        );

        let client = create_mock_client_with_owners(owners);
        let manager = Arc::new(ObjectLockManager::new(client));
        let tx_data = create_test_tx_data(vec![], vec![], vec![shared_id]);

        let locks = manager.try_acquire_locks(1, &tx_data).await.unwrap();
        assert!(locks.locked_objects.is_empty());
    }
}
