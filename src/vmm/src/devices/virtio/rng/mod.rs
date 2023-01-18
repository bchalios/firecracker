// Copyright 2022 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod device;
pub mod event_handler;
pub mod persist;

use versionize::{VersionMap, Versionize, VersionizeError, VersionizeResult};
use versionize_derive::Versionize;

pub use self::device::{Entropy, Error};

pub(crate) const NUM_QUEUES: usize = 3;
pub(crate) const QUEUE_SIZE: u16 = 256;

pub(crate) const RNG_QUEUE: usize = 0;
pub(crate) const LEAK_QUEUE_1: usize = 1;
pub(crate) const LEAK_QUEUE_2: usize = 2;

#[derive(Debug, Versionize, PartialEq, Clone)]
pub(crate) enum LeakQueue {
    LeakQueue1,
    LeakQueue2,
}

impl LeakQueue {
    fn other(&self) -> Self {
        match self {
            LeakQueue::LeakQueue1 => LeakQueue::LeakQueue2,
            LeakQueue::LeakQueue2 => LeakQueue::LeakQueue1,
        }
    }
}

impl From<&LeakQueue> for usize {
    fn from(queue: &LeakQueue) -> Self {
        match queue {
            LeakQueue::LeakQueue1 => LEAK_QUEUE_1,
            LeakQueue::LeakQueue2 => LEAK_QUEUE_2,
        }
    }
}

impl From<LeakQueue> for usize {
    fn from(queue: LeakQueue) -> Self {
        usize::from(&queue)
    }
}
