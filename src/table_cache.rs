// Copyright 2019 Fullstop000 <fullstop1005@gmail.com>.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

// Copyright (c) 2011 The LevelDB Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::storage::Storage;
use std::rc::Rc;
use crate::options::{Options, ReadOptions};
use crate::cache::{Cache, HandleRef};
use crate::sstable::table::{Table, new_table_iterator};
use std::cell::RefCell;
use crate::cache::lru::SharedLRUCache;
use crate::util::status::Result;
use crate::util::varint::VarintU64;
use crate::db::filename::{generate_filename, FileType};
use crate::util::slice::Slice;
use crate::iterator::{Iterator, EmptyIterator, IterWithCleanup, ConcatenateIterator};
use crate::db::format::ParsedInternalKey;
use std::sync::Arc;

/// A `TableCache` is the cache for the sst files and the sstable in them
pub struct TableCache {
    env: Arc<dyn Storage>,
    db_name: String,
    options: Rc<Options>,
    // the key of cache is the file number
    cache: Rc<RefCell<dyn Cache<Rc<Table>>>>,
}

impl TableCache {
    pub fn new(db_name: String, options: Rc<Options>, size: usize) -> Self {
        let cache = Rc::new(RefCell::new(SharedLRUCache::<Rc<Table>>::new(size)));
        Self {
            env: options.env.clone(),
            db_name,
            options,
            cache,
        }
    }

    // Try to find the sst file from cache. If not found, try to find the file from storage and insert it into the cache
    fn find_table(&self, file_number: u64, file_size: u64) -> Result<HandleRef<Rc<Table>>> {
        let mut key = vec![];
        VarintU64::put_varint(&mut key, file_number);
        match self.cache.borrow().look_up(key.as_slice()) {
            Some(handle) => Ok(handle),
            None => {
                let filename = generate_filename(self.db_name.as_str(), FileType::Table, file_number);
                let table_file= self.env.open(filename.as_str())?;
                let table = Table::open(table_file, file_size, self.options.clone())?;
                return Ok(self.cache.borrow_mut().insert(key,  Rc::new(table), 1, None));
            }
        }
    }

    /// Evict any entry for the specified file number
    pub fn evict(&mut self, file_number: u64) {
        let mut key = vec![];
        VarintU64::put_varint(&mut key, file_number);
        self.cache.borrow_mut().erase(key.as_slice());
    }

    /// Returns the result of a seek to internal key `key` in specified file
    pub fn get(&self, options: Rc<ReadOptions>, key: &Slice, file_number: u64, file_size: u64) -> Result<Option<ParsedInternalKey>> {
        let handle = self.find_table(file_number, file_size)?;
        // every value should be valid so unwrap is safe here
        let parsed_key = handle.borrow().get_value().unwrap().internal_get(options, key.as_slice())?;
        self.cache.borrow_mut().release(handle);
        Ok(parsed_key)
    }

    /// Create an iterator for the specified `file_number` (the corresponding
    /// file length must be exactly `file_size` bytes).
    /// The table referenced by returning Iterator will be released after the Iterator is dropped.
    ///
    /// Entry format:
    ///     key: internal key
    ///     value: value of user key
    pub fn new_iter(&self, options: Rc<ReadOptions>, file_number: u64, file_size: u64) -> Box<dyn Iterator> {
        match self.find_table(file_number, file_size) {
            Ok(h) => {
                let table = h.borrow().get_value().unwrap();
                let mut iter = IterWithCleanup::new(new_table_iterator(table, options));
                let cache = self.cache.clone();
                iter.register_task(Box::new(move || cache.borrow_mut().release(h.clone())));
                Box::new(iter)
            }
            Err(e) => EmptyIterator::new_with_err(e)
        }
    }
}