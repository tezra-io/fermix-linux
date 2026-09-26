//! Which microphone a call would record from, kept current while devices come
//! and go. It reads the list from the same sound server the call's `pulsesrc`
//! records through, by GStreamer's device provider for it. Reading the list
//! never opens a device: no recording stream, and no microphone indicator.

use fermix_client::voice::Source;
use gst::prelude::*;
use gtk::glib;

/// The device provider for the sound server the call records through.
pub const SOUND_SERVER: &str = "pulsedeviceprovider";
/// The device class of an input.
const INPUT: &str = "Audio/Source";

/// A running watch. Dropping it stops the provider and its bus watch.
pub struct MicrophoneWatch {
    provider: gst::DeviceProvider,
    _bus: gst::bus::BusWatchGuard,
}

impl MicrophoneWatch {
    /// Starts `provider` and watches its list. `on_change` runs on this thread's
    /// default main context with the whole list of inputs after every device that
    /// comes, goes or changes; `sources` is the list right now. An error means the
    /// list cannot be read, most often because the sound server is out of reach.
    pub fn start(
        provider: &str,
        on_change: impl Fn(Vec<Source>) + 'static,
    ) -> Result<MicrophoneWatch, glib::BoolError> {
        assert!(!provider.is_empty(), "a watch names its device provider");
        gst::init().map_err(|e| glib::bool_error!("GStreamer did not start: {e}"))?;
        let provider = gst::DeviceProviderFactory::by_name(provider)
            .ok_or_else(|| glib::bool_error!("no device provider named {provider}"))?;
        let weak = provider.downgrade();
        let bus = provider.bus().add_watch_local(move |_, message| {
            let change = matches!(
                message.view(),
                gst::MessageView::DeviceAdded(_)
                    | gst::MessageView::DeviceRemoved(_)
                    | gst::MessageView::DeviceChanged(_)
            );
            if let Some(provider) = weak.upgrade().filter(|_| change) {
                on_change(inputs(&provider));
            }
            glib::ControlFlow::Continue
        })?;
        provider.start()?;
        Ok(MicrophoneWatch {
            provider,
            _bus: bus,
        })
    }

    pub fn sources(&self) -> Vec<Source> {
        inputs(&self.provider)
    }
}

impl Drop for MicrophoneWatch {
    fn drop(&mut self) {
        self.provider.stop();
    }
}

/// The provider's inputs; it lists outputs too.
fn inputs(provider: &gst::DeviceProvider) -> Vec<Source> {
    provider
        .devices()
        .iter()
        .filter(|device| device.has_classes(INPUT))
        .map(source)
        .collect()
}

/// The sound server's own words for a device: its class says whether it is a
/// copy of an output, and the provider flags the server's default.
fn source(device: &gst::Device) -> Source {
    let properties = device.properties();
    let class = properties
        .as_ref()
        .and_then(|p| p.get::<String>("device.class").ok());
    let default = properties
        .as_ref()
        .and_then(|p| p.get::<bool>("is-default").ok());
    Source {
        name: device.display_name().to_string(),
        monitor: class.as_deref() == Some("monitor"),
        default: default.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Once;
    use std::time::{Duration, Instant};

    /// Registered once per process; no real device monitor ever picks it (rank none).
    const FAKE: &str = "fermixfakemicrophones";
    const PATIENCE: Duration = Duration::from_secs(5);

    mod imp {
        use gst::subclass::prelude::*;
        use gtk::glib;
        use std::sync::LazyLock;

        #[derive(Default)]
        pub struct FakeProvider;

        #[glib::object_subclass]
        impl ObjectSubclass for FakeProvider {
            const NAME: &'static str = "FermixFakeMicrophones";
            type Type = super::FakeProvider;
            type ParentType = gst::DeviceProvider;
        }

        impl ObjectImpl for FakeProvider {}
        impl GstObjectImpl for FakeProvider {}

        impl DeviceProviderImpl for FakeProvider {
            fn metadata() -> Option<&'static gst::subclass::DeviceProviderMetadata> {
                static METADATA: LazyLock<gst::subclass::DeviceProviderMetadata> =
                    LazyLock::new(|| {
                        gst::subclass::DeviceProviderMetadata::new(
                            "Fake microphones",
                            "Audio/Source",
                            "Devices a test plugs in and takes out",
                            "Fermix",
                        )
                    });
                Some(&METADATA)
            }

            fn start(&self) -> Result<(), gst::LoggableError> {
                Ok(())
            }

            fn stop(&self) {}
        }

        #[derive(Default)]
        pub struct FakeDevice;

        #[glib::object_subclass]
        impl ObjectSubclass for FakeDevice {
            const NAME: &'static str = "FermixFakeMicrophone";
            type Type = super::FakeDevice;
            type ParentType = gst::Device;
        }

        impl ObjectImpl for FakeDevice {}
        impl GstObjectImpl for FakeDevice {}
        impl DeviceImpl for FakeDevice {}
    }

    glib::wrapper! {
        pub struct FakeProvider(ObjectSubclass<imp::FakeProvider>)
            @extends gst::DeviceProvider, gst::Object;
    }

    glib::wrapper! {
        pub struct FakeDevice(ObjectSubclass<imp::FakeDevice>)
            @extends gst::Device, gst::Object;
    }

    fn fake_provider() -> gst::DeviceProvider {
        static REGISTER: Once = Once::new();
        gst::init().expect("GStreamer initialises");
        REGISTER.call_once(|| {
            gst::DeviceProvider::register(None, FAKE, gst::Rank::NONE, FakeProvider::static_type())
                .expect("the fake provider registers");
        });
        gst::DeviceProviderFactory::by_name(FAKE).expect("the fake provider is registered")
    }

    fn device(name: &str, class: &str, properties: gst::Structure) -> gst::Device {
        glib::Object::builder::<FakeDevice>()
            .property("display-name", name)
            .property("device-class", class)
            .property("properties", properties)
            .build()
            .upcast()
    }

    fn microphone(name: &str, default: bool) -> gst::Device {
        let properties = gst::Structure::builder("fake")
            .field("device.class", "sound")
            .field("is-default", default)
            .build();
        device(name, INPUT, properties)
    }

    /// Runs `test` with its own main context as this thread's default, so the
    /// bus watches of tests on parallel threads never share a loop.
    fn on_own_loop(test: impl FnOnce(&glib::MainContext)) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a new context is free to acquire");
    }

    fn run_until(context: &glib::MainContext, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + PATIENCE;
        while !done() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    fn names(sources: &[Source]) -> Vec<(String, bool)> {
        sources
            .iter()
            .map(|s| (s.name.clone(), s.default))
            .collect()
    }

    #[test]
    fn the_watch_follows_microphones_as_they_come_change_and_go() {
        on_own_loop(|context| {
            let provider = fake_provider();
            let seen: Rc<RefCell<Vec<Vec<Source>>>> = Rc::default();
            let record = seen.clone();
            let watch = MicrophoneWatch::start(FAKE, move |list| record.borrow_mut().push(list))
                .expect("the fake provider starts");
            assert!(watch.sources().is_empty());
            // Each change posts one message, and each message is one report: waiting
            // for the count keeps a queued report from standing in for a later one.
            let report = |count: usize| {
                assert!(run_until(context, || seen.borrow().len() >= count));
                assert_eq!(seen.borrow().len(), count, "one report per change");
                names(&seen.borrow()[count - 1])
            };
            // An output on the same server is never an input.
            let speakers = device("Speakers", "Audio/Sink", gst::Structure::new_empty("fake"));
            provider.device_add(&speakers);
            assert_eq!(report(1), Vec::new());
            let usb = microphone("USB Microphone", false);
            provider.device_add(&usb);
            assert_eq!(report(2), vec![("USB Microphone".to_owned(), false)]);
            let usb_default = microphone("USB Microphone", true);
            provider.device_changed(&usb_default, &usb);
            assert_eq!(report(3), vec![("USB Microphone".to_owned(), true)]);
            provider.device_remove(&usb_default);
            assert_eq!(report(4), Vec::new());
            provider.device_remove(&speakers);
            assert_eq!(report(5), Vec::new());
            drop(watch);
            let after = microphone("After", false);
            provider.device_add(&after);
            context.iteration(false);
            assert_eq!(seen.borrow().len(), 5, "a dropped watch reports nothing");
            provider.device_remove(&after);
        });
    }

    #[test]
    fn a_source_carries_the_servers_class_and_default_flag() {
        gst::init().expect("GStreamer initialises");
        let monitor_props = gst::Structure::builder("fake")
            .field("device.class", "monitor")
            .build();
        let monitor = source(&device("Monitor of Speakers", INPUT, monitor_props));
        assert_eq!(
            monitor,
            Source {
                name: "Monitor of Speakers".into(),
                monitor: true,
                default: false,
            }
        );
        let bare = source(&device("Line In", INPUT, gst::Structure::new_empty("fake")));
        assert!(!bare.monitor && !bare.default);
        let default = source(&microphone("fifine Microphone", true));
        assert!(!default.monitor && default.default);
    }

    #[test]
    fn a_provider_that_does_not_exist_is_an_error() {
        let error = MicrophoneWatch::start("fermixnosuchprovider", |_| {});
        assert!(error.is_err());
    }
}
