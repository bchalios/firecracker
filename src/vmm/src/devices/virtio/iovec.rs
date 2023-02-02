// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::marker::PhantomData;

use libc::{c_void, iovec, size_t};
use utils::vm_memory::{Bitmap, GuestMemory, GuestMemoryMmap};

use crate::devices::virtio::DescriptorChain;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// We found a write-only descriptor where read-only was expected
    #[error("Tried to create an `IoVec` from a write-only descriptor chain")]
    WriteOnlyDescriptor,
    /// We found a read-only descriptor where write-only was expected
    #[error("Tried to create an 'IoVecMut` from a read-only descriptor chain")]
    ReadOnlyDescriptor,
    /// An error happened with guest memory handling
    #[error("Guest memory error: {0}")]
    GuestMemory(#[from] utils::vm_memory::GuestMemoryError),
}

type Result<T> = std::result::Result<T, Error>;

// Describes a sub-region of a buffer described as a slice of `iovec` structs.
#[derive(Debug)]
struct IoVecSubregion<'a> {
    // An iterator of the iovec items we are iterating
    iovecs: Vec<iovec>,
    // Lifetime of the origin buffer
    phantom: PhantomData<&'a iovec>,
}

impl<'a> IoVecSubregion<'a> {
    // Create a new `IoVecSubregion`
    //
    // Given an initial buffer (described as a collecetion of `iovec` structs) and a sub-region
    // inside it, in the form of [offset; size] create a "sub-region" inside it, if the sub-region
    // does not fall outside the original buffer, i.e. `offset` is not after the end of the original
    // buffer.
    //
    // # Arguments
    //
    // * `iovecs` - A slice of `iovec` structures describing the buffer.
    // * `len`    - The total length of the buffer, i.e. the sum of the lengths of all `iovec`
    //   structs.
    // * `offset` - The offset inside the buffer at which the sub-region starts.
    // * `size`   - The size of the sub-region
    //
    // # Returns
    //
    // If the sub-region is within the range of the buffer, i.e. the offset is not past the end of
    // the buffer, it will return an `IoVecSubregion`.
    fn new(iovecs: &'a [iovec], len: usize, mut offset: usize, mut size: usize) -> Option<Self> {
        // Out-of-bounds sub-region
        if offset >= len {
            return None;
        }

        // Empty sub-region
        if size == 0 {
            return None;
        }

        let sub_regions = iovecs
            .iter()
            .filter_map(|iov| {
                // If offset is bigger than the length of the current `iovec`, this `iovec` is not
                // part of the sub-range
                if offset >= iov.iov_len {
                    offset -= iov.iov_len;
                    return None;
                }

                // No more `iovec` structs needed
                if size == 0 {
                    return None;
                }

                // SAFETY: This is safe because we chacked that `offset < iov.iov_len`.
                let iov_base = unsafe { iov.iov_base.add(offset) };
                let iov_len = std::cmp::min(iov.iov_len - offset, size);
                offset = 0;
                size -= iov_len;

                Some(iovec { iov_base, iov_len })
            })
            .collect();

        Some(Self {
            iovecs: sub_regions,
            phantom: PhantomData,
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.iovecs.iter().fold(0, |acc, iov| acc + iov.iov_len)
    }
}

impl<'a> IntoIterator for IoVecSubregion<'a> {
    type Item = iovec;

    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.iovecs.into_iter()
    }
}

// Create a `libc::iovec` from a `DescriptorChain`
//
// This will make sure that the address region `[desc.addr, desc.addr + desc.len)` is
// valid guest memory.
fn iovec_try_from_descriptor_chain(
    mem: &GuestMemoryMmap,
    desc: &DescriptorChain,
    write_only: bool,
) -> Result<iovec> {
    // We use `get_slice` instead of `get_host_address` here in order to have the whole
    // range of the descriptor chain checked, i.e. [addr, addr + len) is a valid memory
    // region in the GuestMemoryMmap.
    let slice = mem.get_slice(desc.addr, desc.len as usize)?;

    // We need to mark the area of guest memory that will be mutated through this
    // IoVecBuffer as dirty ahead of time, as we loose access to all
    // vm-memory related information after convering down to iovecs.
    if write_only {
        slice.bitmap().mark_dirty(0, desc.len as usize);
    }

    let iov_base = slice.as_ptr().cast::<c_void>();

    Ok(iovec {
        iov_base,
        iov_len: desc.len as size_t,
    })
}

/// `IoVecBuffer` describes one or more buffers provided to us by the guest. Buffers provided to us
/// by the guest can be scattered across memory. `IoVecBuffer` parses the descriptors of provided
/// buffers and provides an interface to read from or write to arbitrary ranges within these
/// scattered buffers.
///
/// A buffer provided to us by the guest consists of zero or more read-only physically contiguous
/// elements, followed by zero or more write-only physically contiguous elements.
#[derive(Debug, Default)]
pub(crate) struct IoVecBuffer<'a> {
    // descriptor id of the last parster DescriptorChain
    desc_id: Option<u16>,
    // container of the memory regions included in this IO vector
    vecs: Vec<iovec>,
    // Length in bytes of read-only part
    read_len: usize,
    // Length in bytes of write-only part
    write_len: usize,
    // Offset of write-only iovecs in `vec`
    split: usize,
    // PhantomData that make the buffer valid for the lifetime of the GuestMemoryMmap
    // object they were created from
    phantom: PhantomData<&'a GuestMemoryMmap>,
}

impl<'a> IoVecBuffer<'a> {
    /// Create a new, empty, `IoVecBuffer`
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.desc_id = None;
        self.vecs.clear();
        self.read_len = 0;
        self.write_len = 0;
        self.split = 0;
    }

    /// Parse a new `DescriptorChain` in the `IoVecBuffer`
    ///
    /// This parses a `DescriptorChain` that consists of one or more buffers. read-only parts of
    /// the sequence are first in the chain, followed by write-only parts.
    ///
    /// # Arguments
    ///
    /// * `mem` - The guest memory mmap object
    /// * `head` - The head of the descriptor chain passed to us by the guest
    ///
    /// # Returns
    ///
    /// This will return an error if:
    ///
    /// * One of the descriptors passed describes invalid guest memory
    /// * A read-only descriptor is found after a write-only descriptor
    pub(crate) fn parse(&mut self, mem: &GuestMemoryMmap, head: DescriptorChain) -> Result<()> {
        self.clear();
        self.desc_id = Some(head.index);

        let mut head_iter = head.into_iter().peekable();

        // Parse read-only part
        while let Some(desc) = head_iter.next_if(|next| !next.is_write_only()) {
            // It's ok to unwrap because `desc.len` is a `u32` which in our supported architectures
            // should fit in a `usize`.
            self.read_len += usize::try_from(desc.len).unwrap();
            self.vecs
                .push(iovec_try_from_descriptor_chain(mem, &desc, false)?);
        }

        self.split = self.vecs.len();

        // Parse write-only part
        for desc in head_iter {
            if !desc.is_write_only() {
                return Err(Error::ReadOnlyDescriptor);
            }

            // It's ok to unwrap because `desc.len` is a `u32` which in our supported architectures
            // should fit in a `usize`.
            self.write_len += usize::try_from(desc.len).unwrap();
            self.vecs
                .push(iovec_try_from_descriptor_chain(mem, &desc, true)?);
        }

        Ok(())
    }

    /// Parse a read-only `DescriptorChain` in the `IoVecBuffer`.
    ///
    /// # Arguments
    ///
    /// * `mem` - The guest memory mmap object
    /// * `head` - The head of the descriptor chain passed to us by the guest
    ///
    /// # Returns
    ///
    /// This will return an error if:
    ///
    /// * One of the descriptors passed describes invalid guest memory
    /// * A write-only descriptor is found in the chain
    pub(crate) fn parse_read_only(
        &mut self,
        mem: &GuestMemoryMmap,
        head: DescriptorChain,
    ) -> Result<()> {
        self.clear();
        self.desc_id = Some(head.index);

        for desc in head {
            if desc.is_write_only() {
                return Err(Error::WriteOnlyDescriptor);
            }

            self.read_len += usize::try_from(desc.len).unwrap();
            self.vecs
                .push(iovec_try_from_descriptor_chain(mem, &desc, false)?);
        }

        self.split = self.vecs.len();

        Ok(())
    }

    /// Parse a write-only `DescriptorChain` in the `IoVecBuffer`.
    ///
    /// # Arguments
    ///
    /// * `mem` - The guest memory mmap object
    /// * `head` - The head of the descriptor chain passed to us by the guest
    ///
    /// # Returns
    ///
    /// This will return an error if:
    ///
    /// * One of the descriptors passed describes invalid guest memory
    /// * A read-only descriptor is found in the chain
    pub(crate) fn parse_write_only(
        &mut self,
        mem: &GuestMemoryMmap,
        head: DescriptorChain,
    ) -> Result<()> {
        self.clear();
        self.desc_id = Some(head.index);

        for desc in head {
            if !desc.is_write_only() {
                return Err(Error::ReadOnlyDescriptor);
            }

            self.write_len += usize::try_from(desc.len).unwrap();
            self.vecs
                .push(iovec_try_from_descriptor_chain(mem, &desc, true)?);
        }

        Ok(())
    }

    pub(crate) fn descriptor_id(&self) -> Option<u16> {
        self.desc_id
    }

    /// Get a slice to the read-only part of the chain.
    pub(crate) fn read(&self) -> &[iovec] {
        &self.vecs[..self.split]
    }

    /// Get a slice to the write-only part of the chain.
    pub(crate) fn write(&self) -> &[iovec] {
        &self.vecs[self.split..]
    }

    fn read_subregion(&self, offset: usize, size: usize) -> Option<IoVecSubregion> {
        IoVecSubregion::new(self.read(), self.read_len, offset, size)
    }

    fn write_subregion(&self, offset: usize, size: usize) -> Option<IoVecSubregion> {
        IoVecSubregion::new(self.write(), self.write_len, offset, size)
    }

    /// Reads a number of bytes from the `IoVecBuffer` starting at a given offset.
    ///
    /// This will try to fill `buf` reading bytes from the `IoVecBuffer` starting from
    /// the given offset. It will read as many bytes from `IoVecBuffer` starting from `offset` as
    /// they in ``
    ///
    /// # Arguments
    ///
    /// * `buf` - The slice in which we will read bytes.
    /// * `offset` - The offset within the (read part) of the `IoVecBuffer` from which we will start
    ///   reading bytes.
    ///
    /// # Returns
    ///
    /// The number of bytes read (if any)
    pub(crate) fn read_at(&self, buf: &mut [u8], offset: usize) -> Option<usize> {
        self.read_subregion(offset, buf.len()).map(|sub_region| {
            let mut bytes = 0;
            let mut buf_ptr = buf.as_mut_ptr();

            sub_region.into_iter().for_each(|iov| {
                let src = iov.iov_base.cast::<u8>();
                // SAFETY:
                // The call to `copy_nonoverlapping` is safe because:
                // 1. `iov` is a an iovec describing a segment inside `Self`. `IoVecSubregion` has
                //    performed all necessary bound checks.
                // 2. `buf_ptr` is a pointer inside the memory of `buf`
                // 3. Both pointers point to `u8` elements, so they're always aligned.
                // 4. The memory regions these pointers point to are not overlapping. `src` points
                //    to guest physical memory and `buf_ptr` to Firecracker-owned memory.
                //
                // `buf_ptr.add()` is safe because `IoVecSubregion` gives us `iovec` structs that
                // their size adds up to `buf.len()`.
                unsafe {
                    std::ptr::copy_nonoverlapping(src, buf_ptr, iov.iov_len);
                    buf_ptr = buf_ptr.add(iov.iov_len);
                }
                bytes += iov.iov_len;
            });

            bytes
        })
    }

    /// Writes a number of bytes into the `IoVecBuffer` starting at a given offset.
    ///
    /// This will try to fill `IoVecBuffer` writing bytes from the `buf` starting from
    /// the given offset. It will write as many bytes from `buf` as they fit inside the
    /// `IoVecBuffer` starting from `offset`.
    ///
    /// # Arguments
    ///
    /// * `buf` - The slice in which we will read bytes.
    /// * `offset` - The offset within the (read part) of the `IoVecBuffer` from which we will start
    ///   reading bytes.
    ///
    /// # Returns
    ///
    /// The number of bytes written (if any)
    pub fn write_at(&mut self, buf: &[u8], offset: usize) -> Option<usize> {
        self.write_subregion(offset, buf.len()).map(|sub_region| {
            let mut bytes = 0;
            let mut buf_ptr = buf.as_ptr();

            sub_region.into_iter().for_each(|iov| {
                let dst = iov.iov_base.cast::<u8>();
                // SAFETY:
                // The call to `copy_nonoverlapping` is safe because:
                // 1. `iov` is a an iovec describing a segment inside `Self`. `IoVecSubregion` has
                //    performed all necessary bound checks.
                // 2. `buf_ptr` is a pointer inside the memory of `buf`
                // 3. Both pointers point to `u8` elements, so they're always aligned.
                // 4. The memory regions these pointers point to are not overlapping. `src` points
                //    to guest physical memory and `buf_ptr` to Firecracker-owned memory.
                //
                // `buf_ptr.add()` is safe because `IoVecSubregion` gives us `iovec` structs that
                // their size adds up to `buf.len()`.
                unsafe {
                    std::ptr::copy_nonoverlapping(buf_ptr, dst, iov.iov_len);
                    buf_ptr = buf_ptr.add(iov.iov_len);
                }
                bytes += iov.iov_len;
            });

            bytes
        })
    }

    /// Length of read-only part
    pub(crate) fn read_len(&self) -> usize {
        self.read_len
    }

    /// Length of write-only part
    pub(crate) fn write_len(&self) -> usize {
        self.write_len
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;

    use libc::{c_void, iovec};
    use utils::vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

    use super::IoVecBuffer;
    use crate::devices::virtio::queue::VIRTQ_DESC_F_WRITE;
    use crate::devices::virtio::test_utils::test::{create_virtio_mem, VirtioTestTransport};

    impl<'a> From<Vec<&[u8]>> for IoVecBuffer<'a> {
        fn from(data: Vec<&[u8]>) -> Self {
            let mut read_len = 0usize;
            let vecs = data
                .iter()
                .map(|slice| {
                    read_len += slice.len();
                    iovec {
                        iov_base: slice.as_ptr() as *mut c_void,
                        iov_len: slice.len(),
                    }
                })
                .collect();

            IoVecBuffer {
                desc_id: Some(42),
                vecs,
                read_len,
                write_len: 0,
                split: data.len(),
                phantom: PhantomData,
            }
        }
    }

    fn add_read_only_chain<'a>(mem: &'a GuestMemoryMmap, transport: &mut VirtioTestTransport<'a>) {
        let v: Vec<u8> = (0..=255).collect();
        mem.write_slice(&v, GuestAddress(transport.data_address()))
            .unwrap();
        transport.add_desc_chain(0, 0, &[(0, 64, 0), (1, 64, 0), (2, 64, 0), (3, 64, 0)]);
    }

    #[test]
    fn test_parse_read_only() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();

        // Add a read-only buffer
        transport.add_desc_chain(0, 0, &[(0, 64, 0), (1, 64, 0)]);
        // Add a read-write buffer
        transport.add_desc_chain(0, 128, &[(2, 64, 0), (3, 64, VIRTQ_DESC_F_WRITE)]);
        // Add a write-only buffer
        transport.add_desc_chain(
            0,
            128,
            &[(4, 64, VIRTQ_DESC_F_WRITE), (5, 64, VIRTQ_DESC_F_WRITE)],
        );

        let mut iovec = IoVecBuffer::new();
        // First descriptor chain is read-only
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_read_only(&mem, head).is_ok());
        assert_eq!(iovec.descriptor_id(), Some(0));
        assert_eq!(iovec.vecs.len(), 2);
        assert_eq!(iovec.read_len(), 128);
        assert_eq!(iovec.write_len(), 0);
        assert_eq!(iovec.split, iovec.vecs.len());

        // That's a read-write chain
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_read_only(&mem, head).is_err());

        // That's a write-only chain
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_read_only(&mem, head).is_err());
    }

    #[test]
    fn test_parse_write_only() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();

        // Add a read-only buffer
        transport.add_desc_chain(0, 0, &[(0, 64, 0), (1, 64, 0)]);
        // Add a read-write buffer
        transport.add_desc_chain(0, 128, &[(2, 64, 0), (3, 64, VIRTQ_DESC_F_WRITE)]);
        // Add a write-only buffer
        transport.add_desc_chain(
            0,
            128,
            &[(4, 64, VIRTQ_DESC_F_WRITE), (5, 64, VIRTQ_DESC_F_WRITE)],
        );

        let mut iovec = IoVecBuffer::new();
        // First descriptor chain is read-only
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_write_only(&mem, head).is_err());

        // That's a read-write chain
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_write_only(&mem, head).is_err());

        // That's a write-only chain
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse_write_only(&mem, head).is_ok());
        assert_eq!(iovec.descriptor_id(), Some(4));
        assert_eq!(iovec.vecs.len(), 2);
        assert_eq!(iovec.write_len(), 128);
        assert_eq!(iovec.read_len(), 0);
        assert_eq!(iovec.split, 0);
    }

    #[test]
    fn test_parse_read_write() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();

        // Add a read-only buffer
        transport.add_desc_chain(0, 0, &[(0, 64, 0), (1, 64, 0)]);
        // Add a read-write buffer
        transport.add_desc_chain(0, 128, &[(2, 64, 0), (3, 64, VIRTQ_DESC_F_WRITE)]);
        // Add a write-only buffer
        transport.add_desc_chain(
            0,
            128,
            &[(4, 64, VIRTQ_DESC_F_WRITE), (5, 64, VIRTQ_DESC_F_WRITE)],
        );

        let mut iovec = IoVecBuffer::new();
        // First descriptor chain is read-only
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse(&mem, head).is_ok());
        assert_eq!(iovec.descriptor_id(), Some(0));
        assert_eq!(iovec.vecs.len(), 2);
        assert_eq!(iovec.read_len(), 128);
        assert_eq!(iovec.write_len(), 0);
        assert_eq!(iovec.split, iovec.vecs.len());

        // This is a read-write descriptor
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse(&mem, head).is_ok());
        assert_eq!(iovec.descriptor_id(), Some(2));
        assert_eq!(iovec.vecs.len(), 2);
        assert_eq!(iovec.read_len(), 64);
        assert_eq!(iovec.read_len(), 64);
        assert_eq!(iovec.split, 1);

        // And finally a write-only descriptor
        let head = queue[0].pop(&mem).unwrap();
        assert!(iovec.parse(&mem, head).is_ok());
        assert_eq!(iovec.descriptor_id(), Some(4));
        assert_eq!(iovec.vecs.len(), 2);
        assert_eq!(iovec.read_len(), 0);
        assert_eq!(iovec.write_len(), 128);
        assert_eq!(iovec.split, 0);
    }

    fn read_tests(iovec: &IoVecBuffer) {
        let mut buf = vec![0; 5];
        assert_eq!(iovec.read_at(&mut buf[..4], 0), Some(4));
        assert_eq!(buf, vec![0u8, 1, 2, 3, 0]);

        assert_eq!(iovec.read_at(&mut buf, 0), Some(5));
        assert_eq!(buf, vec![0u8, 1, 2, 3, 4]);

        assert_eq!(iovec.read_at(&mut buf, 1), Some(5));
        assert_eq!(buf, vec![1u8, 2, 3, 4, 5]);

        assert_eq!(iovec.read_at(&mut buf, 60), Some(5));
        assert_eq!(buf, vec![60u8, 61, 62, 63, 64]);

        assert_eq!(iovec.read_at(&mut buf, 252), Some(4));
        assert_eq!(buf[0..4], vec![252u8, 253, 254, 255]);

        assert_eq!(iovec.read_at(&mut buf, 256), None);
    }

    #[test]
    fn test_read_only_iovec_read_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        add_read_only_chain(&mem, &mut transport);
        let mut queue = transport.create_queues();

        let head = queue[0].pop(&mem).unwrap();
        let mut iovec = IoVecBuffer::new();
        iovec.parse_read_only(&mem, head).unwrap();

        read_tests(&iovec);
    }

    #[test]
    fn test_read_write_iovec_read_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        add_read_only_chain(&mem, &mut transport);
        let mut queue = transport.create_queues();

        let head = queue[0].pop(&mem).unwrap();
        let mut iovec = IoVecBuffer::new();
        iovec.parse(&mem, head).unwrap();

        read_tests(&iovec);
    }

    #[test]
    #[should_panic]
    fn test_write_only_iovec_read_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();

        transport.add_desc_chain(
            0,
            0,
            &[
                (0, 64, VIRTQ_DESC_F_WRITE),
                (1, 64, VIRTQ_DESC_F_WRITE),
                (2, 64, VIRTQ_DESC_F_WRITE),
                (3, 64, VIRTQ_DESC_F_WRITE),
            ],
        );

        let head = queue[0].pop(&mem).unwrap();
        let mut iovec = IoVecBuffer::new();
        iovec.parse(&mem, head).unwrap();

        read_tests(&iovec);
    }

    fn write_tests(iovec: &mut IoVecBuffer, transport: &mut VirtioTestTransport) {
        // One test vector for each part of the chain
        let mut test_vec1 = vec![0u8; 64];
        let mut test_vec2 = vec![0u8; 64];
        let test_vec3 = vec![0u8; 64];
        let mut test_vec4 = vec![0u8; 64];

        // Control test: Initially all four regions should be zero
        assert_eq!(iovec.write_at(&test_vec1, 0), Some(64));
        assert_eq!(iovec.write_at(&test_vec2, 64), Some(64));
        assert_eq!(iovec.write_at(&test_vec3, 128), Some(64));
        assert_eq!(iovec.write_at(&test_vec4, 192), Some(64));
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );

        // Let's initialize test_vec1 with our buffer.
        let buf = vec![0u8, 1, 2, 3, 4];
        test_vec1[..buf.len()].copy_from_slice(&buf);
        // And write just a part of it
        assert_eq!(iovec.write_at(&buf[..3], 0), Some(3));
        // Not all 5 bytes from buf should be written in memory,
        // just 3 of them.
        transport.check_data(
            0,
            &[
                (0, &[0u8, 1, 2, 0, 0]),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );
        // But if we write the whole `buf` in memory then all
        // of it should be observable.
        assert_eq!(iovec.write_at(&buf, 0), Some(5));
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );

        // We are now writing with an offset of 1. So, initialize
        // the corresponding part of `test_vec1`
        test_vec1[1..buf.len() + 1].copy_from_slice(&buf);
        assert_eq!(iovec.write_at(&buf, 1), Some(5));
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );

        // Perform a write that traverses two of the underlying
        // regions. Writing at offset 60 should write 4 bytes on the
        // first region and one byte on the second
        test_vec1[60..64].copy_from_slice(&buf[0..4]);
        test_vec2[0] = 4;
        assert_eq!(iovec.write_at(&buf, 60), Some(5));
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );

        test_vec4[63] = 3;
        test_vec4[62] = 2;
        test_vec4[61] = 1;
        // Now perform a write that does not fit in the buffer. Try writing
        // 5 bytes at offset 252 (only 4 bytes left).
        test_vec4[60..64].copy_from_slice(&buf[0..4]);
        assert_eq!(iovec.write_at(&buf, 252), Some(4));
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );

        // Trying to add past the end of the buffer should not write anything
        assert_eq!(iovec.write_at(&buf, 256), None);
        transport.check_data(
            0,
            &[
                (0, &test_vec1),
                (1, &test_vec2),
                (2, &test_vec3),
                (3, &test_vec4),
            ],
        );
    }

    #[test]
    fn test_write_only_iovec_write_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();
        transport.add_desc_chain(
            0,
            0,
            &[
                (0, 64, VIRTQ_DESC_F_WRITE),
                (1, 64, VIRTQ_DESC_F_WRITE),
                (2, 64, VIRTQ_DESC_F_WRITE),
                (3, 64, VIRTQ_DESC_F_WRITE),
            ],
        );

        // This is a descriptor chain with 4 elements 64 bytes long each.
        let head = queue[0].pop(&mem).unwrap();

        let mut iovec = IoVecBuffer::new();
        iovec.parse_write_only(&mem, head).unwrap();

        write_tests(&mut iovec, &mut transport);
    }

    #[test]
    fn test_read_write_iovec_write_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();
        transport.add_desc_chain(
            0,
            0,
            &[
                (0, 64, VIRTQ_DESC_F_WRITE),
                (1, 64, VIRTQ_DESC_F_WRITE),
                (2, 64, VIRTQ_DESC_F_WRITE),
                (3, 64, VIRTQ_DESC_F_WRITE),
            ],
        );

        // This is a descriptor chain with 4 elements 64 bytes long each.
        let head = queue[0].pop(&mem).unwrap();

        let mut iovec = IoVecBuffer::new();
        iovec.parse(&mem, head).unwrap();

        write_tests(&mut iovec, &mut transport);
    }

    #[test]
    #[should_panic]
    fn test_read_only_iovec_write_at() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        let mut queue = transport.create_queues();
        transport.add_desc_chain(0, 0, &[(0, 64, 0), (1, 64, 0), (2, 64, 0), (3, 64, 0)]);

        // This is a descriptor chain with 4 elements 64 bytes long each.
        let head = queue[0].pop(&mem).unwrap();

        let mut iovec = IoVecBuffer::new();
        iovec.parse(&mem, head).unwrap();

        write_tests(&mut iovec, &mut transport);
    }

    #[test]
    fn test_sub_range() {
        let mem = create_virtio_mem();
        let mut transport = VirtioTestTransport::new(&mem, 1, 8);
        add_read_only_chain(&mem, &mut transport);
        let mut queue = transport.create_queues();
        let head = queue[0].pop(&mem).unwrap();

        let mut iovec = IoVecBuffer::new();
        // This is a descriptor chain with 4 elements 64 bytes long each,
        // so 256 bytes long.
        iovec.parse(&mem, head).unwrap();

        // Sub-ranges past the end of the buffer are invalid
        assert!(iovec.read_subregion(iovec.read_len(), 256).is_none());

        // Getting an empty sub-range is invalid
        assert!(iovec.read_subregion(0, 0).is_none());

        // Let's take the whole region
        let sub = iovec.read_subregion(0, iovec.read_len()).unwrap();
        assert_eq!(iovec.read_len(), sub.len());

        // Let's take a valid sub-region that ends past the the end of the buffer
        let sub = iovec.read_subregion(128, 256).unwrap();
        assert_eq!(128, sub.len());

        // Getting a sub-region that falls in a single iovec of the buffer
        for i in 0..4 {
            let sub = iovec.read_subregion(10 + i * 64, 50).unwrap();
            assert_eq!(50, sub.len());
            assert_eq!(1, sub.iovecs.len());
            // SAFETY: All `iovecs` are 64 bytes long
            assert_eq!(sub.iovecs[0].iov_base, unsafe {
                iovec.vecs[i].iov_base.add(10)
            });
        }

        // Get a sub-region that traverses more than one iovec of the buffer
        let sub = iovec.read_subregion(10, 100).unwrap();
        assert_eq!(100, sub.len());
        assert_eq!(2, sub.iovecs.len());
        // SAFETY: all `iovecs` are 64 bytes long
        assert_eq!(sub.iovecs[0].iov_base, unsafe {
            iovec.vecs[0].iov_base.add(10)
        });

        assert_eq!(sub.iovecs[1].iov_base, iovec.vecs[1].iov_base);
    }
}
