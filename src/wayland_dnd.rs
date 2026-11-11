//! Drag and drop of files onto the window on native Wayland.
//!
//! winit (0.30) delivers dropped files on X11 only; on Wayland a drop from
//! a file manager is simply ignored. This module fills the gap: it attaches
//! a second event queue to the display connection the window already uses,
//! binds `wl_data_device_manager` there and accepts `text/uri-list` drops.
//!
//! Nothing here reads the socket. winit's event loop does that; the events
//! for our objects are sorted into our queue by libwayland and are handled
//! when [`WaylandDrop::poll`] runs once per UI frame.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Read as _;
use std::os::fd::AsFd as _;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};

use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const URI_LIST: &str = "text/uri-list";
/// `wl_data_offer.set_actions` and `.finish` exist since version 3.
const ACTIONS_SINCE: u32 = 3;
/// A drop is a list of file names; anything larger is not one.
const MAX_DROP_BYTES: u64 = 4 * 1024 * 1024;

/// Receives file drops. Create once per window, poll once per frame.
pub struct WaylandDrop {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    dropped: Receiver<String>,
    failed: bool,
}

struct State {
    manager: Option<WlDataDeviceManager>,
    /// Seats seen so far, with the registry name they were announced under.
    seats: Vec<(u32, WlSeat)>,
    devices: Vec<(u32, WlDataDevice)>,
    /// MIME types announced for each offer that is still alive.
    offers: HashMap<ObjectId, Vec<String>>,
    /// The offer of the drag currently over the window, if it carries files.
    current: Option<WlDataOffer>,
    hovering: bool,
    sender: Sender<String>,
    repaint: egui::Context,
}

impl WaylandDrop {
    /// Attach to the window's display connection.
    ///
    /// # Safety
    ///
    /// `display` must be the live `wl_display` of the window this value is
    /// used with, and the value must be dropped before that display is
    /// disconnected (the application keeps it for as long as the window).
    pub unsafe fn new(display: NonNull<c_void>, repaint: egui::Context) -> Self {
        // SAFETY: guaranteed by the caller.
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let _ = connection.flush();

        let (sender, dropped) = mpsc::channel();
        Self {
            connection,
            queue,
            state: State {
                manager: None,
                seats: Vec::new(),
                devices: Vec::new(),
                offers: HashMap::new(),
                current: None,
                hovering: false,
                sender,
                repaint,
            },
            dropped,
            failed: false,
        }
    }

    /// Handle pending events and return the `text/uri-list` payloads of
    /// drops that have completed since the last call.
    pub fn poll(&mut self) -> Vec<String> {
        if !self.failed {
            let result = self
                .connection
                .backend()
                .dispatch_inner_queue()
                .map_err(|err| err.to_string())
                .and_then(|_| {
                    self.queue
                        .dispatch_pending(&mut self.state)
                        .map_err(|err| err.to_string())
                });
            if let Err(err) = result {
                tracing::warn!(%err, "Wayland drag and drop stopped working");
                self.failed = true;
            }
            let _ = self.connection.flush();
        }
        self.dropped.try_iter().collect()
    }

    /// A drag carrying files is over the window right now.
    pub fn hovering(&self) -> bool {
        self.state.hovering
    }
}

impl State {
    /// Create a data device for every seat that does not have one yet.
    fn bind_devices(&mut self, queue: &QueueHandle<Self>) {
        let Some(manager) = &self.manager else { return };
        for (name, seat) in &self.seats {
            if !self.devices.iter().any(|(owner, _)| owner == name) {
                self.devices
                    .push((*name, manager.get_data_device(seat, queue, ())));
            }
        }
    }

    fn forget(&mut self, offer: &WlDataOffer) {
        self.offers.remove(&offer.id());
        offer.destroy();
    }

    fn leave(&mut self) {
        if let Some(offer) = self.current.take() {
            self.forget(&offer);
        }
        if self.hovering {
            self.hovering = false;
            self.repaint.request_repaint();
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        queue: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                match interface.as_str() {
                    "wl_data_device_manager" if state.manager.is_none() => {
                        state.manager = Some(registry.bind(name, version.min(3), queue, ()));
                    }
                    "wl_seat" => state.seats.push((name, registry.bind(name, 1, queue, ()))),
                    _ => return,
                }
                state.bind_devices(queue);
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.seats.retain(|(seat, _)| *seat != name);
                state.devices.retain(|(seat, device)| {
                    if *seat == name {
                        device.release();
                    }
                    *seat != name
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some(types) = state.offers.get_mut(&offer.id())
        {
            types.push(mime_type);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Enter { serial, id, .. } => {
                state.leave();
                let Some(offer) = id else { return };
                let has_files = state
                    .offers
                    .get(&offer.id())
                    .is_some_and(|types| types.iter().any(|mime| mime == URI_LIST));
                if !has_files {
                    offer.accept(serial, None);
                    state.forget(&offer);
                    return;
                }
                offer.accept(serial, Some(URI_LIST.to_owned()));
                if offer.version() >= ACTIONS_SINCE {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
                state.current = Some(offer);
                state.hovering = true;
                state.repaint.request_repaint();
            }
            wl_data_device::Event::Leave => state.leave(),
            wl_data_device::Event::Drop => {
                let Some(offer) = state.current.take() else {
                    return;
                };
                state.offers.remove(&offer.id());
                state.hovering = false;
                receive(
                    offer,
                    connection,
                    state.sender.clone(),
                    state.repaint.clone(),
                );
            }
            // The clipboard arrives on the same device. It is not used
            // here (egui has its own clipboard), so the offer is released.
            wl_data_device::Event::Selection { id: Some(offer) } => state.forget(&offer),
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

/// Ask the drag source for the URI list and read it on a worker thread: the
/// source writes whenever it gets round to it, and the UI must not wait.
fn receive(
    offer: WlDataOffer,
    connection: &Connection,
    sender: Sender<String>,
    repaint: egui::Context,
) {
    let finish = {
        let connection = connection.clone();
        move |offer: WlDataOffer, accepted: bool| {
            if accepted && offer.version() >= ACTIONS_SINCE {
                offer.finish();
            }
            offer.destroy();
            let _ = connection.flush();
        }
    };

    let (reader, writer) = match std::io::pipe() {
        Ok(pipe) => pipe,
        Err(err) => {
            tracing::warn!(%err, "could not create a pipe for the dropped files");
            finish(offer, false);
            return;
        }
    };
    offer.receive(URI_LIST.to_owned(), writer.as_fd());
    let _ = connection.flush();
    // Our copy of the write end must be closed, or the read never ends.
    drop(writer);

    let spawned = std::thread::Builder::new()
        .name("wayland-drop".to_owned())
        .spawn(move || {
            let mut text = String::new();
            let read = reader.take(MAX_DROP_BYTES).read_to_string(&mut text);
            finish(offer, read.is_ok());
            match read {
                Ok(_) => {
                    let _ = sender.send(text);
                    repaint.request_repaint();
                }
                Err(err) => tracing::warn!(%err, "could not read the dropped files"),
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not start the drop reader thread");
    }
}
