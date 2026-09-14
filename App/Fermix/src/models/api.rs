//! What a model is allowed to ask the daemon, and the bounds it asks within.
//!
//! One trait, two methods. The real implementation is the framed socket client;
//! the other is an in-memory peer answering the vendored goldens, so a model
//! test proves the model rather than the socket. Everything above this file
//! speaks typed requests and typed results: the `serde_json::Value` in the
//! middle exists so the trait is two methods wide rather than forty.

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::management::types::HelloResult;
use crate::management::{Issued, ManagementClient, ManagementError};

/// One in-flight exchange.
pub type Answer<'a, T> = Pin<Box<dyn Future<Output = Issued<Result<T, ManagementError>>> + 'a>>;

/// How long a read may take. One deadline covers the whole exchange.
pub const READ_DEADLINE: Duration = Duration::from_secs(10);
/// How long a write may take. Longer than a read: a write can touch a keyring
/// or rewrite the settings file before it answers.
pub const WRITE_DEADLINE: Duration = Duration::from_secs(20);

/// At most this many reads are in flight at once. A refresh that would exceed
/// it is coalesced away rather than queued, because a superseded read has
/// nothing to say that the next one will not.
pub const MAX_CONCURRENT_READS: usize = 4;
/// At most this much work is outstanding at once. Past it a write is refused as
/// busy rather than dropped, because dropping a write loses what a person
/// typed.
pub const MAX_QUEUED: usize = 16;

/// The daemon, as a model sees it.
pub trait ManagementApi {
    /// The current connection epoch. An answer issued under an older one can
    /// never reach the model.
    fn epoch(&self) -> u64;

    /// Drop the session: a new epoch, and the negotiation taken again.
    fn reset(&self);

    /// The version both halves settled on, once `hello` has run.
    fn negotiated_version(&self) -> Option<u32>;

    /// The last `hello` answer, if one has landed under the current epoch.
    fn last_hello(&self) -> Option<HelloResult>;

    /// Negotiate, and answer with the daemon's own identity.
    fn hello(&self, deadline: Duration) -> Answer<'_, HelloResult>;

    /// Point this client at the socket the command line reported, and say
    /// whether that moved it. A peer that is not a socket answers `false`.
    fn rebind(&self, socket: &std::path::Path) -> bool;

    /// One call, by method name, with the parameters the method publishes.
    fn call(
        &self,
        method: &str,
        params: serde_json::Value,
        deadline: Duration,
    ) -> Answer<'_, serde_json::Value>;
}

impl ManagementApi for ManagementClient {
    fn epoch(&self) -> u64 {
        ManagementClient::epoch(self)
    }

    fn reset(&self) {
        ManagementClient::reset(self)
    }

    fn negotiated_version(&self) -> Option<u32> {
        ManagementClient::negotiated_version(self)
    }

    fn last_hello(&self) -> Option<HelloResult> {
        ManagementClient::last_hello(self)
    }

    fn hello(&self, deadline: Duration) -> Answer<'_, HelloResult> {
        Box::pin(async move { ManagementClient::hello(self, deadline).await })
    }

    fn rebind(&self, socket: &std::path::Path) -> bool {
        ManagementClient::rebind(self, socket)
    }

    fn call(
        &self,
        method: &str,
        params: serde_json::Value,
        deadline: Duration,
    ) -> Answer<'_, serde_json::Value> {
        let method = method.to_string();
        Box::pin(async move { self.request(&method, &params, deadline).await })
    }
}

/// One typed call: typed parameters in, a typed result out, stamped with the
/// epoch it was issued under.
pub async fn ask<P, R>(
    api: &dyn ManagementApi,
    method: &str,
    params: &P,
    deadline: Duration,
) -> Issued<Result<R, ManagementError>>
where
    P: Serialize,
    R: DeserializeOwned,
{
    let epoch = api.epoch();

    let encoded = match serde_json::to_value(params) {
        Ok(encoded) => encoded,
        Err(error) => {
            return Issued {
                epoch,
                value: Err(ManagementError::Decode {
                    method: method.to_string(),
                    reason: error.to_string(),
                }),
            }
        }
    };

    let issued = api.call(method, encoded, deadline).await;
    Issued {
        epoch: issued.epoch,
        value: issued.value.and_then(|result| {
            serde_json::from_value(result).map_err(|error| ManagementError::Decode {
                method: method.to_string(),
                reason: error.to_string(),
            })
        }),
    }
}

/// The answer, or nothing when the connection epoch moved while it was in
/// flight. Every model result goes through here.
pub fn accept<T>(api: &dyn ManagementApi, issued: Issued<T>) -> Option<T> {
    if api.epoch() == issued.epoch {
        Some(issued.value)
    } else {
        None
    }
}

/// The bounds of M38 section 12.2, counted.
///
/// Reads coalesce at their ceiling; everything else is refused as busy rather
/// than discarded. A permit gives the count back on every path, including a
/// refusal and a deadline, because it is released when it is dropped.
#[derive(Default)]
pub struct Gate {
    reads: Rc<Cell<usize>>,
    outstanding: Rc<Cell<usize>>,
    lifecycle: Rc<Cell<bool>>,
}

/// One outstanding operation. Dropping it releases the count.
pub struct Permit {
    outstanding: Rc<Cell<usize>>,
    reads: Option<Rc<Cell<usize>>>,
    lifecycle: Option<Rc<Cell<bool>>>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.outstanding
            .set(self.outstanding.get().saturating_sub(1));
        if let Some(reads) = &self.reads {
            reads.set(reads.get().saturating_sub(1));
        }
        if let Some(lifecycle) = &self.lifecycle {
            lifecycle.set(false);
        }
    }
}

impl Gate {
    /// A permit for one read, or nothing when four are already in flight.
    pub fn read(&self) -> Option<Permit> {
        if self.reads.get() >= MAX_CONCURRENT_READS || self.outstanding.get() >= MAX_QUEUED {
            return None;
        }
        self.reads.set(self.reads.get() + 1);
        self.outstanding.set(self.outstanding.get() + 1);

        Some(Permit {
            outstanding: Rc::clone(&self.outstanding),
            reads: Some(Rc::clone(&self.reads)),
            lifecycle: None,
        })
    }

    /// A permit for one write, or nothing when sixteen operations are already
    /// outstanding.
    pub fn write(&self) -> Option<Permit> {
        if self.outstanding.get() >= MAX_QUEUED {
            return None;
        }
        self.outstanding.set(self.outstanding.get() + 1);

        Some(Permit {
            outstanding: Rc::clone(&self.outstanding),
            reads: None,
            lifecycle: None,
        })
    }

    /// A permit for the one lifecycle mutation, or nothing while one is
    /// running. Installing, uninstalling and restarting share the slot: two at
    /// once would race over the same unit.
    pub fn lifecycle(&self) -> Option<Permit> {
        if self.lifecycle.get() || self.outstanding.get() >= MAX_QUEUED {
            return None;
        }
        self.lifecycle.set(true);
        self.outstanding.set(self.outstanding.get() + 1);

        Some(Permit {
            outstanding: Rc::clone(&self.outstanding),
            reads: None,
            lifecycle: Some(Rc::clone(&self.lifecycle)),
        })
    }

    /// How much work is outstanding.
    pub fn outstanding(&self) -> usize {
        self.outstanding.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fifth_concurrent_read_is_coalesced_away() {
        let gate = Gate::default();
        let held: Vec<Permit> = (0..MAX_CONCURRENT_READS)
            .map(|_| gate.read().expect("a read permit"))
            .collect();

        assert!(gate.read().is_none());
        drop(held);
        assert!(gate.read().is_some());
    }

    #[test]
    fn a_permit_gives_its_count_back_when_it_is_dropped() {
        let gate = Gate::default();
        {
            let _permit = gate.read().expect("a read permit");
            assert_eq!(gate.outstanding(), 1);
        }
        assert_eq!(gate.outstanding(), 0);
    }

    #[test]
    fn only_one_lifecycle_mutation_runs_at_a_time() {
        let gate = Gate::default();
        let held = gate.lifecycle().expect("a lifecycle permit");

        assert!(gate.lifecycle().is_none());
        drop(held);
        assert!(gate.lifecycle().is_some());
    }

    #[test]
    fn work_past_the_queue_ceiling_is_refused_rather_than_discarded() {
        let gate = Gate::default();
        let held: Vec<Permit> = (0..MAX_QUEUED)
            .map(|_| gate.write().expect("a write permit"))
            .collect();

        assert!(gate.write().is_none());
        assert!(gate.read().is_none());
        drop(held);
        assert!(gate.write().is_some());
    }
}
