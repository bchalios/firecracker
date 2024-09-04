use std::os::fd::AsRawFd;

use libc::{c_int, c_void, iovec, off_t, size_t};
use memfd;

use crate::arch::PAGE_SIZE;

use super::queue::FIRECRACKER_MAX_QUEUE_SIZE;

#[derive(Debug)]
pub(crate) struct IovDeque<'a> {
    iov: &'a mut [libc::iovec],
    head: usize,
    tail: usize,
}

// SAFETY: TODO
unsafe impl<'a> Send for IovDeque<'a> {}

#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum IovDequeError {
    /// Error with [`Memfd`]
    Memfd(#[from] memfd::Error),
    /// Error while resizing ['Memfd']
    MemfdResize(std::io::Error),
    /// Error with `mmap`
    Mmap(std::io::Error),
    /// IovDeque is full
    Full,
    /// IovDeque is empty
    Empty,
}

impl<'a> IovDeque<'a> {
    fn create_memfd() -> Result<memfd::Memfd, IovDequeError> {
        // Create a sealable memfd.
        let opts = memfd::MemfdOptions::default().allow_sealing(true);
        let mfd = opts.create("sized-1K")?;

        // Resize to 1024B.
        mfd.as_file()
            .set_len(PAGE_SIZE.try_into().unwrap())
            .map_err(IovDequeError::MemfdResize)?;

        // Add seals to prevent further resizing.
        mfd.add_seals(&[memfd::FileSeal::SealShrink, memfd::FileSeal::SealGrow])?;

        // Prevent further sealing changes.
        mfd.add_seal(memfd::FileSeal::SealSeal)?;

        Ok(mfd)
    }

    fn do_mmap(
        addr: *mut c_void,
        len: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: off_t,
    ) -> Result<*mut c_void, IovDequeError> {
        // SAFETY: We are calling the system call with valid arguments and properly checking its
        // return value
        let ptr = unsafe { libc::mmap(addr, len, prot, flags, fd, offset) };
        if ptr == libc::MAP_FAILED {
            return Err(IovDequeError::Mmap(std::io::Error::last_os_error()));
        }

        Ok(ptr)
    }

    fn allocate_memory() -> Result<*mut c_void, IovDequeError> {
        Self::do_mmap(
            std::ptr::null_mut(),
            PAGE_SIZE * 2,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    }

    pub(crate) fn new() -> Result<Self, IovDequeError> {
        let memfd = Self::create_memfd()?;

        let raw_memfd = memfd.as_file().as_raw_fd();
        let buffer = Self::allocate_memory()?;

        let _ = Self::do_mmap(
            buffer,
            PAGE_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED | libc::MAP_FIXED,
            raw_memfd,
            0,
        )?;

        // SAFETY: safe because `Self::allocate_memory` allocates exactly two pages for us
        let next_page = unsafe { buffer.add(PAGE_SIZE) };
        let _ = Self::do_mmap(
            next_page,
            PAGE_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED | libc::MAP_FIXED,
            raw_memfd,
            0,
        )?;

        // SAFETY:
        // * `buffer` is valid both for reads and writes (allocated with `libc::PROT_READ |
        //    libc::PROT_WRITE`. `
        // * `buffer` is aligned at `PAGE_SIZE`
        // * `buffer` points to memory allocated with a single system call to `libc::mmap`
        let iov = unsafe {
            std::slice::from_raw_parts_mut(
                buffer.cast(),
                2 * PAGE_SIZE / std::mem::size_of::<libc::iovec>(),
            )
        };

        // TODO: explain why this is fine
        std::mem::forget(memfd);

        Ok(Self {
            iov,
            head: 0,
            tail: 0,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.tail - self.head
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub(crate) fn push_back(&mut self, iov: iovec) -> Result<(), IovDequeError> {
        if self.tail - self.head == usize::from(FIRECRACKER_MAX_QUEUE_SIZE) {
            return Err(IovDequeError::Full);
        }

        self.iov[self.tail] = iov;
        self.tail += 1;

        Ok(())
    }

    pub(crate) fn pop_front(&mut self) -> Result<iovec, IovDequeError> {
        if self.is_empty() {
            return Err(IovDequeError::Empty);
        }

        let iov = self.iov[self.head];
        self.head += 1;
        if self.head > usize::from(FIRECRACKER_MAX_QUEUE_SIZE) {
            self.head -= usize::from(FIRECRACKER_MAX_QUEUE_SIZE);
            self.tail -= usize::from(FIRECRACKER_MAX_QUEUE_SIZE);
        }

        Ok(iov)
    }

    pub(crate) fn drop_iovs(&mut self, size: usize) -> usize {
        let mut dropped = 0usize;

        while dropped < size {
            if self.iov.is_empty() {
                return 0;
            }

            let iov = self.pop_front().unwrap();
            dropped += iov.iov_len;
        }

        dropped
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [iovec] {
        &mut self.iov[self.head..self.tail]
    }

    pub(crate) fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
    }
}

#[cfg(test)]
mod tests {
    use libc::iovec;

    use crate::devices::virtio::iov_deque::IovDequeError;

    use super::IovDeque;

    #[test]
    fn test_new() {
        let iov = IovDeque::new().unwrap();
        assert!(iov.is_empty());
    }

    fn make_iovec(seed: usize) -> iovec {
        iovec {
            iov_base: seed as *mut libc::c_void,
            iov_len: seed,
        }
    }

    #[test]
    fn test_push_back() {
        let mut iov = IovDeque::new().unwrap();
        assert!(iov.is_empty());

        for i in 0usize..256 {
            iov.push_back(make_iovec(i)).unwrap();
            assert_eq!(iov.len(), i + 1);
        }

        assert!(matches!(
            iov.push_back(make_iovec(0)),
            Err(IovDequeError::Full)
        ));
    }

    #[test]
    fn test_pop() {
        let mut deque = IovDeque::new().unwrap();
        assert!(deque.is_empty());

        assert!(matches!(deque.pop_front(), Err(IovDequeError::Empty)));

        for i in 0usize..256 {
            deque.push_back(make_iovec(i)).unwrap();
            assert_eq!(deque.len(), i + 1);
        }

        for i in 0usize..256 {
            let iov = deque.pop_front().unwrap();
            assert_eq!(iov.iov_base, i as *mut libc::c_void);
            assert_eq!(iov.iov_len, i);
        }
    }
}
