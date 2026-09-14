//! The models.
//!
//! One settings model, built in the application composition, and a narrow model
//! per surface that reads it rather than keeping a copy of what it holds. Every
//! model runs on the GLib main context: nothing here starts a thread, and every
//! repeated observation goes through the one bounded poller below.

pub mod activation;
pub mod api;
pub mod computer;
pub mod doctor;
pub mod home;
pub mod jobs;
pub mod ledger;
pub mod logs;
pub mod meetings;
pub mod notices;
pub mod onboarding;
pub mod pane;
pub mod peer;
pub mod plugins;
pub mod providers;
pub mod recovery;
pub mod settings_model;

pub use settings_model::{Change, RowId, Sentence, SettingsModel, State};

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;

/// Run one future on the main context.
///
/// Every model action is an ordinary `async fn`, so a test can await it
/// directly; this is how a view starts one without importing the toolkit's
/// scheduler at every call site.
pub fn spawn(future: impl std::future::Future<Output = ()> + 'static) {
    glib::spawn_future_local(future);
}

/// A list of callbacks one feature model tells when it changes.
///
/// The settings model publishes a typed change; a feature model owns its own
/// reading of one surface, so what it publishes is "something moved". The view
/// reads the model back rather than being handed a payload it could hold onto.
/// One callback.
type Observer = Rc<dyn Fn()>;

#[derive(Default, Clone)]
pub struct Observers {
    list: Rc<RefCell<Vec<Observer>>>,
}

impl Observers {
    /// Add one. It runs on the main context.
    pub fn add(&self, observer: impl Fn() + 'static) {
        self.list.borrow_mut().push(Rc::new(observer));
    }

    /// Tell them all, without holding a borrow while they run.
    pub fn notify(&self) {
        let observers: Vec<Observer> = self.list.borrow().clone();
        for observer in observers {
            observer();
        }
    }
}

/// A repeated observation, bounded.
///
/// Every poll in the application runs through one of these: it counts its own
/// ticks, stops at a declared cap, and is stopped when the surface that owns it
/// goes away. A poller that outlives its view is the defect this exists to make
/// impossible.
///
/// It waits on the context the caller is already on, rather than arming a
/// source on the process's default one: `timeout_add_local` takes that single
/// context whichever thread calls it, so two of these started at the same
/// moment on two threads is a panic rather than a wait. The window has one
/// context and never noticed; a test binary runs its tests on several threads
/// and does.
#[derive(Default, Clone)]
pub struct Poller {
    inner: Rc<Inner>,
}

#[derive(Default)]
struct Inner {
    task: RefCell<Option<glib::JoinHandle<()>>>,
    running: Cell<bool>,
    ticks: Cell<u32>,
}

impl Poller {
    /// A poller that is not running.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start polling, replacing whatever was running.
    ///
    /// `cap` is the most ticks this poller will ever perform. Reaching it stops
    /// the poller, which is a visible timeout rather than a poll that runs for
    /// the life of the process.
    pub fn start(&self, interval: Duration, cap: u32, tick: impl Fn() -> bool + 'static) {
        self.stop();
        self.inner.ticks.set(0);
        self.inner.running.set(true);

        let inner = Rc::clone(&self.inner);
        let task = glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(interval).await;
                inner.ticks.set(inner.ticks.get().saturating_add(1));

                if !(tick() && inner.ticks.get() < cap) {
                    // Marked here rather than in `stop`: a poller that reached
                    // its own cap has stopped, and the caller reads that back.
                    inner.running.set(false);
                    return;
                }
            }
        });

        self.inner.task.replace(Some(task));
    }

    /// Stop polling. Doing this twice is not an error.
    pub fn stop(&self) {
        self.inner.running.set(false);
        if let Some(task) = self.inner.task.replace(None) {
            task.abort();
        }
    }

    /// Whether this poller is running.
    pub fn is_running(&self) -> bool {
        self.inner.running.get()
    }

    /// How many times it has ticked since it was started.
    pub fn ticks(&self) -> u32 {
        self.inner.ticks.get()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.running.set(false);
        if let Some(task) = self.task.replace(None) {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::MainContext;

    /// Each of these runs on a context of its own, pushed as this thread's
    /// default for as long as it runs, which is where the poller it starts
    /// waits. Nothing here touches the process's default context, so these
    /// tests take no turns with the rest of the binary.
    #[test]
    fn a_poll_is_bounded_replaceable_and_stoppable() {
        on_its_own_context(stops_at_its_cap);
        on_its_own_context(a_tick_that_answers_no_ends_it);
        on_its_own_context(starting_again_replaces_it);
        stopping_one_that_never_started_is_not_an_error();
    }

    fn on_its_own_context(body: impl FnOnce(&MainContext)) {
        let context = MainContext::new();
        context
            .clone()
            .with_thread_default(|| body(&context))
            .expect("the context is this thread's default while the test runs");
    }

    /// Run the context until a condition holds or a bound is reached, so a test
    /// never waits on a tick that is not coming.
    fn pump(context: &MainContext, done: impl Fn() -> bool) {
        for _ in 0..2_000 {
            if done() {
                return;
            }
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn counting(poller: &Poller, interval: Duration, cap: u32) -> Rc<Cell<u32>> {
        let counted = Rc::new(Cell::new(0u32));
        let owned = Rc::clone(&counted);
        poller.start(interval, cap, move || {
            owned.set(owned.get() + 1);
            true
        });
        counted
    }

    fn stops_at_its_cap(context: &MainContext) {
        let poller = Poller::new();
        let counted = counting(&poller, Duration::from_millis(1), 3);

        pump(context, || !poller.is_running());

        assert_eq!(counted.get(), 3, "the cap is the most it ever ticks");
        assert!(!poller.is_running());
    }

    fn a_tick_that_answers_no_ends_it(context: &MainContext) {
        let counted = Rc::new(Cell::new(0u32));
        let poller = Poller::new();
        {
            let counted = Rc::clone(&counted);
            poller.start(Duration::from_millis(1), 100, move || {
                counted.set(counted.get() + 1);
                false
            });
        }

        pump(context, || !poller.is_running());

        assert_eq!(counted.get(), 1);
        assert!(!poller.is_running());
    }

    fn starting_again_replaces_it(context: &MainContext) {
        let poller = Poller::new();
        let first = counting(&poller, Duration::from_millis(1), 100);
        let second = counting(&poller, Duration::from_millis(1), 2);

        pump(context, || !poller.is_running());

        assert_eq!(second.get(), 2);
        assert_eq!(first.get(), 0, "the replaced poll never ran");
    }

    fn stopping_one_that_never_started_is_not_an_error() {
        let poller = Poller::new();
        poller.stop();
        poller.stop();
        assert!(!poller.is_running());
        assert_eq!(poller.ticks(), 0);
    }
}
