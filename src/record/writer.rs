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

use std::fs::File;
use std::io::{Result, Seek, SeekFrom, Write};
use crate::util::slice::Slice;
use crate::record::{RecordType, BLOCK_SIZE, HEADER_SIZE};
use crate::util::crc32;
use std::mem;
use crate::util::coding::encode_fixed_32;

/// Writer writes records to an underlying log `File`.
pub struct Writer {
    dest: File,
    //Current offset in block
    block_offset: usize,
    // crc32c values for all supported record types.  These are
    // pre-computed to reduce the overhead of computing the crc of the
    // record type stored in the header.
    crc_cache: [u32; (RecordType::Last as usize + 1) as usize]
}

impl Writer {
    pub fn new(mut dest: File) -> Result<Self> {
        let offset = dest.seek(SeekFrom::Current(0))?;
        let n = RecordType::Last as usize;
        let mut cache = [0; RecordType::Last as usize + 1];
        for h in 0..n +1 {
            let v: [u8; 1] = unsafe { mem::transmute(RecordType::from(h) as u8) };
            cache[h as usize] = crc32::value(&v);
        }
        let w = Writer {
            dest,
            block_offset: offset as usize % BLOCK_SIZE,
            crc_cache: cache,
        };
        Ok(w)
    }

    /// Appends a slice into the underlying log file
    pub fn add_record(&mut self, s: &Slice) -> Result<()> {
        let data = s.to_slice();
        let mut left = s.size();
        let mut begin = true; // indicate the record should be a
        while left > 0 {
            invarint!(
                BLOCK_SIZE >= self.block_offset,
                "[record writer] the 'block_offset' {} overflows the max BLOCK_SIZE {}",
                self.block_offset, BLOCK_SIZE,
            );
            let leftover = BLOCK_SIZE - self.block_offset;

            // switch to a new block if the left size is not enough
            // for a record header
            if leftover < HEADER_SIZE {
                if leftover != 0 {
                    // fill the rest of the block with zero
                    self.dest.write_all(&[0;6][..leftover])?;
                }
                self.block_offset = 0; // use a new block
            };
            invarint!(
                BLOCK_SIZE >= self.block_offset + HEADER_SIZE,
                "[record writer] the left space of block {} is less than header size {}",
                BLOCK_SIZE - self.block_offset, HEADER_SIZE,
            );
            let space = BLOCK_SIZE - self.block_offset - HEADER_SIZE;
            let to_write = if left < space {
                left
            } else {
                space
            };
            // indicates iff the data exhausts a record
            let end = to_write == left;
            let t = {
                if begin && end {
                    RecordType::Full
                } else if begin {
                    RecordType::First
                } else if end {
                    RecordType::Last
                } else {
                    RecordType::Middle
                }
            };
            self.write(t, &data[..to_write])?;
            left -= to_write;
            begin = false;
        };
        Ok(())

    }

    // create formatted bytes and write into the file
    fn write(&mut self, rt: RecordType, data: &[u8]) -> Result<()> {
        let size = data.len();
        invarint!(
            size <= 0xffff,
            "[record writer] the data length in a record must fit 2 bytes but got {}",
            size
        );
        invarint!(
            self.block_offset + HEADER_SIZE + size <= BLOCK_SIZE,
            "[record writer] new record [{:?}] overflows the BLOCK_SIZE [{}]",
            rt, BLOCK_SIZE,
        );
        // encode header
        let mut buf: [u8; HEADER_SIZE] = [0; HEADER_SIZE];
        buf[4] = (size & 0xff) as u8; // data length
        buf[5] = (size >> 8) as u8;
        buf[6] = rt as u8; // record type

        // encode crc
        let mut crc = crc32::extend(self.crc_cache[rt as usize], data);
        crc = crc32::mask(crc);
        encode_fixed_32(&mut buf, crc);

        // write the header and the data
        self.dest.write_all(&buf)?;
        self.dest.write_all(data)?;
        self.dest.flush()?;
        // update block_offset
        self.block_offset += HEADER_SIZE + size;
        Ok(())
    }
}