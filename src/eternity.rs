use hash_hasher::HashedMap;

use crate::*;

#[path = "epoch.rs"]
mod epoch;
pub use epoch::{Block, BlockMut, Epoch, EpochMut};

/// A sparse commitment tree to witness up to 65,536 [`Epoch`]s, each witnessing up to 65,536
/// [`Block`]s, each witnessing up to 65,536 [`Fq`]s or their [`struct@Hash`]es.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Eternity {
    epoch_index: HashedMap<Fq, index::Epoch>,
    block_index: HashedMap<Fq, index::Block>,
    item_index: HashedMap<Fq, index::Item>,
    inner: Tier<Tier<Tier<Item>>>,
}

impl Height for Eternity {
    type Height = <Tier<Tier<Tier<Item>>> as Height>::Height;
}

impl Eternity {
    /// Create a new empty [`Eternity`] for storing all commitments to the end of time.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new [`Epoch`] (or its root hash) all at once to this [`Eternity`].
    ///
    /// # Errors
    ///
    /// Returns `Err(epoch)` without adding it to the [`Eternity`] if the [`Eternity`] is full.
    pub fn insert(&mut self, epoch: Insert<Epoch>) -> Result<(), Insert<Epoch>> {
        // TODO: deal with duplicates

        // If we successfully insert this epoch, here's what its index in the epoch will be:
        let epoch_index = self.inner.len().into();

        // Decompose the epoch into its components
        let (epoch, block_index, item_index) = match epoch {
            Insert::Hash(hash) => (Insert::Hash(hash), Default::default(), Default::default()),
            Insert::Keep(Epoch {
                inner,
                block_index,
                item_index,
            }) => (Insert::Keep(inner), block_index, item_index),
        };

        // Try to insert the epoch into the tree, and if successful, track the item, block, and
        // epoch indices of each item in the epoch
        if let Err(epoch) = self.inner.insert(epoch) {
            Err(epoch.map(|inner| Epoch {
                block_index,
                item_index,
                inner,
            }))
        } else {
            // Keep track of the epoch index of each item in the epoch (these are all the same, all
            // pointing to this epoch we just inserted)
            self.epoch_index
                .extend(item_index.iter().map(|(item, _)| (*item, epoch_index)));
            // Keep track of the block index of each block within its own epoch
            self.block_index.extend(block_index.iter());
            // Keep track of the index within its own block of each item in the block
            self.item_index.extend(item_index.iter());
            Ok(())
        }
    }

    /// The total number of [`Fq`]s or [`struct@Hash`]es represented in this [`Epoch`].
    ///
    /// This count includes those which were elided due to a partially filled [`Block`] or
    /// [`Epoch`], or summary root [`struct@Hash`] of a block or epoch being inserted.
    ///
    /// In other words, this is `2 ^ 32` times the number of epochs represented in this
    /// [`Eternity`], plus `4 ^ 8` times the number of blocks represented in this [`Eternity`],
    /// plus the number of items in the latest block.
    ///
    /// The maximum capacity of an [`Eternity`] is `2 ^ 48`, i.e. `4 ^ 8` epochs of `4 ^ 8` blocks
    /// of `4 ^ 8` items.
    pub fn len(&self) -> u64 {
        ((self.inner.len() as u64) << 32)
            + (match self.inner.focus() {
                None => 0,
                Some(Insert::Hash(_)) => u32::MAX,
                Some(Insert::Keep(epoch)) => {
                    (match epoch.focus() {
                        None => 0,
                        Some(Insert::Hash(_)) => u16::MAX,
                        Some(Insert::Keep(block)) => block.len(),
                    }) as u32
                }
            } << 16) as u64
    }

    /// Check whether this [`Eternity`] is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the root [`struct@Hash`] of this [`Eternity`].
    ///
    /// Internal hashing is performed lazily to prevent unnecessary intermediary hashes from being
    /// computed, so the first hash returned after a long sequence of insertions may take more time
    /// than subsequent calls.
    ///
    /// Computed hashes are cached so that subsequent calls without further modification are very
    /// fast.
    pub fn hash(&self) -> Hash {
        self.inner.hash()
    }

    /// Get a [`Proof`] of inclusion for the item at this index in the eternity.
    ///
    /// If the index is not witnessed in this eternity, return `None`.
    pub fn witness(&self, item: Fq) -> Option<Proof<Eternity>> {
        // Calculate the index for this item
        let this_epoch = *self.epoch_index.get(&item)?;
        let this_block = *self
            .block_index
            .get(&item)
            .expect("if item is present in the epoch index, it must be present in the block index");
        let this_item = *self
            .item_index
            .get(&item)
            .expect("if item is present in block index, it must be present in item index");
        let index = index::within::Eternity {
            epoch: this_epoch,
            block: this_block,
            item: this_item,
        };

        let (auth_path, leaf) = self.inner.witness(index)?;
        debug_assert_eq!(leaf, Hash::of(item));

        Some(Proof {
            index: index.into(),
            auth_path,
            leaf: item,
        })
    }

    /// Forget about the witness for the given [`Fq`].
    ///
    /// Returns `true` if the item was previously witnessed (and now is forgotten), and `false` if
    /// it was not witnessed.
    pub fn forget(&mut self, item: Fq) -> bool {
        if let Some(this_item) = self.item_index.get(&item) {
            let this_block = *self
                .block_index
                .get(&item)
                .expect("if item index contains item, then block index must contain item");

            let this_epoch = *self
                .epoch_index
                .get(&item)
                .expect("if item index contains item, then epoch index must contain item");

            // Calculate the index for the item
            let index = index::within::Eternity {
                item: *this_item,
                block: this_block,
                epoch: this_epoch,
            };

            // Forget the item from the inner tree
            let forgotten = self.inner.forget(index);

            // The index should never contain things that aren't witnessed
            debug_assert!(forgotten, "indexed item must be witnessed in tree");

            // Remove the item from all indices
            self.item_index.remove(&item);
            self.block_index.remove(&item);
            self.epoch_index.remove(&item);

            // The item was indeed previously present, now forgotten
            true
        } else {
            false
        }
    }
}
