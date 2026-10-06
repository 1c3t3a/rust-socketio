use std::time::Duration;

use crate::asynchronous::client::callback::Callback;
use tokio::time::Instant;

use super::callback::DynAsyncCallback;

/// Represents an `Ack` as given back to the caller. Holds the internal `id` as
/// well as the current ack'ed state. Holds data which will be accessible as
/// soon as the ack'ed state is set to true. An `Ack` that didn't get ack'ed
/// won't contain data.
#[derive(Debug)]
pub(crate) struct Ack {
    pub id: i32,
    pub timeout: Duration,
    pub time_started: Instant,
    pub callback: Callback<DynAsyncCallback>,
}

impl Ack {
    // The caller holds the registry lock until this ID has been inserted.
    pub(super) fn next_id(acks: &[Self], mut candidate: i32) -> i32 {
        while acks.iter().any(|ack| ack.id == candidate) {
            candidate = if candidate == i32::MAX {
                0
            } else {
                candidate + 1
            };
        }
        candidate
    }
    pub(super) fn take_matching(acks: &mut Vec<Self>, id: i32) -> Vec<Self> {
        let mut matching = Vec::new();
        let mut index = 0;
        while index < acks.len() {
            if acks[index].id == id {
                matching.push(acks.remove(index));
            } else {
                index += 1;
            }
        }
        matching
    }
}
#[cfg(test)]
mod regression_tests {
    use super::*;
    fn ack(id: i32) -> Ack {
        Ack {
            id,
            timeout: Duration::from_secs(60),
            time_started: Instant::now(),
            callback: Callback::<DynAsyncCallback>::new(|_, _| Box::pin(async {})),
        }
    }
    #[test]
    fn occupied_id_is_not_reused() {
        let mut pending = vec![ack(i32::MAX), ack(0), ack(1)];
        let id = Ack::next_id(&pending, i32::MAX);
        assert!(!pending.iter().any(|ack| ack.id == id));
        pending.push(ack(id));
        let next = Ack::next_id(&pending, i32::MAX);
        assert!(!pending.iter().any(|ack| ack.id == next));
    }
    #[test]
    fn duplicate_ack_removes_all_matches_and_preserves_other_requests() {
        let mut pending = vec![ack(7), ack(8), ack(7)];
        let matches = Ack::take_matching(&mut pending, 7);
        assert_eq!(
            matches.iter().map(|ack| ack.id).collect::<Vec<_>>(),
            vec![7, 7]
        );
        assert_eq!(
            pending.iter().map(|ack| ack.id).collect::<Vec<_>>(),
            vec![8]
        );
        assert!(Ack::take_matching(&mut pending, 7).is_empty());
    }
}
