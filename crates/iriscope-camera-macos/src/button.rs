//! Optional status endpoint reader. Never detaches Apple's video driver.

use crate::{
    button_protocol::{ButtonProtocol, de400_location},
    events::EventMailbox,
};
use iriscope_core::camera::CameraEvent;
use nusb::{
    Endpoint, MaybeFuture,
    transfer::{In, Interrupt},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(super) struct ButtonReceiver {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ButtonReceiver {
    pub(super) fn start(unique_id: &str, events: Arc<EventMailbox>) -> Option<Self> {
        let location = de400_location(unique_id)?;
        events.set_button_status("Bouton DE400 : ouverture de la réception USB macOS…".to_owned());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let failure_events = Arc::clone(&events);
        match thread::Builder::new().name("iriscope-mac-button".to_owned()).spawn(move || {
            match open_endpoint(location) {
                Ok(endpoint) => read_packets(endpoint,&thread_stop,&events),
                Err(error) => events.set_button_status(format!("Bouton DE400 indisponible sur ce Mac : {error}. La capture à l’écran reste disponible.")),
            }
        }) {
            Ok(worker) => Some(Self {stop,worker:Some(worker)}),
            Err(error) => {
                failure_events.set_button_status(format!("Bouton DE400 : impossible de démarrer la lecture USB : {error}"));
                None
            }
        }
    }
}
impl Drop for ButtonReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn open_endpoint(location: u32) -> Result<Endpoint<Interrupt, In>, String> {
    let info = nusb::list_devices()
        .wait()
        .map_err(|e| e.to_string())?
        .find(|d| {
            d.vendor_id() == 0x21cd && d.product_id() == 0x603b && d.location_id() == location
        })
        .ok_or_else(|| "appareil USB correspondant à la caméra introuvable".to_owned())?;
    let number = info
        .interfaces()
        .find(|i| i.class() == 0x0e && i.subclass() == 1)
        .map(nusb::InterfaceInfo::interface_number)
        .ok_or_else(|| "interface de contrôle vidéo introuvable".to_owned())?;
    let device = info
        .open()
        .wait()
        .map_err(|e| format!("accès USB refusé ({e})"))?;
    // claim_interface uses ordinary open; no seize, driver detach, reset or configuration write.
    let interface = device
        .claim_interface(number)
        .wait()
        .map_err(|e| format!("le système refuse l’interface du bouton ({e})"))?;
    interface
        .endpoint::<Interrupt, In>(0x81)
        .map_err(|e| format!("endpoint de statut 0x81 inaccessible ({e})"))
}

fn read_packets(mut endpoint: Endpoint<Interrupt, In>, stop: &AtomicBool, events: &EventMailbox) {
    let packet_size = endpoint.max_packet_size();
    if !(4..=1024).contains(&packet_size) {
        events.set_button_status("Bouton DE400 : taille de paquet USB inattendue.".to_owned());
        return;
    }
    let mut protocol = ButtonProtocol::default();
    // Two queued reads prevent a quick press/release from falling between submissions.
    for _ in 0..2 {
        endpoint.submit(endpoint.allocate(packet_size));
    }
    events.set_button_status(
        "Bouton DE400 : réception USB macOS active. Une pression suit le mode Photo ou Vidéo."
            .to_owned(),
    );
    while !stop.load(Ordering::Acquire) {
        let Some(completion) = endpoint.wait_next_complete(Duration::from_millis(25)) else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        if let Err(error) = completion.status {
            events.set_button_status(format!("Bouton DE400 : lecture USB interrompue ({error}). Rouvrez la caméra pour réessayer."));
            break;
        }
        if protocol.pressed(&completion.buffer) {
            events.publish(Ok(CameraEvent::HardwareButtonPressed));
        }
        endpoint.submit(endpoint.allocate(packet_size));
    }
    // Endpoint drop cancels pending reads before releasing the control interface.
}
