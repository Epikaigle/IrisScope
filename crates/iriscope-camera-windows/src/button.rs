//! Optional standard UVC trigger notification on the existing MF source.
//!
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/ksevent-vidcaptosti-ext-trigger>
//! <https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/sample-user-mode-code-for-methods-and-events>
//!
//! Registration can be rejected by the driver. No second camera, USB interface
//! or replacement driver is opened. Native DE400 verification is still required.

use std::{mem::size_of, ptr};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        Media::{
            KernelStreaming::{
                IKsControl, KSEVENT_TYPE_ENABLE, KSEVENT_VIDCAPTOSTI_EXT_TRIGGER, KSEVENTDATA,
                KSEVENTDATA_0, KSEVENTDATA_0_0, KSEVENTF_EVENT_HANDLE, KSEVENTSETID_VIDCAPTOSTI,
                KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0,
            },
            MediaFoundation::IMFMediaSource,
        },
        System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject},
    },
    core::Interface,
};

/// All registration and cleanup calls remain on the source's COM worker.
pub(super) struct NativeButton {
    control: IKsControl,
    event: HANDLE,
    data: Box<KSEVENTDATA>,
    data_len: u32,
}

impl NativeButton {
    pub(super) fn discover(source: &IMFMediaSource) -> Result<Self, String> {
        let control: IKsControl = source
            .cast()
            .map_err(|error| format!("interface IKsControl indisponible ({error})"))?;
        let request_len = size_of::<KSIDENTIFIER>()
            .try_into()
            .map_err(|error| format!("requête d’événement trop grande ({error})"))?;
        let data_len = size_of::<KSEVENTDATA>()
            .try_into()
            .map_err(|error| format!("données d’événement trop grandes ({error})"))?;
        // SAFETY: Unnamed, initially unsignaled auto-reset event, no security
        // descriptor or shared/global handle. Owned until Drop.
        let event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|error| format!("création de l’événement impossible ({error})"))?;
        let mut data = Box::new(KSEVENTDATA {
            NotificationType: KSEVENTF_EVENT_HANDLE,
            Anonymous: KSEVENTDATA_0 {
                EventHandle: KSEVENTDATA_0_0 {
                    Event: event,
                    Reserved: [0; 2],
                },
            },
        });
        let request = KSIDENTIFIER {
            Anonymous: KSIDENTIFIER_0 {
                Anonymous: KSIDENTIFIER_0_0 {
                    Set: KSEVENTSETID_VIDCAPTOSTI,
                    Id: KSEVENT_VIDCAPTOSTI_EXT_TRIGGER.0.cast_unsigned(),
                    Flags: KSEVENT_TYPE_ENABLE,
                },
            },
        };
        let mut returned = 0;
        // SAFETY: Correctly sized native structures. The boxed event data and
        // event handle remain valid until the subscription is disabled in Drop.
        let registered = unsafe {
            control.KsEvent(
                &raw const request,
                request_len,
                ptr::from_mut(data.as_mut()).cast(),
                data_len,
                &raw mut returned,
            )
        };
        if let Err(error) = registered {
            // SAFETY: No successful subscription retains this owned handle.
            let _ = unsafe { CloseHandle(event) };
            return Err(format!("abonnement au déclencheur refusé ({error})"));
        }
        Ok(Self {
            control,
            event,
            data,
            data_len,
        })
    }

    pub(super) fn take_press(&self) -> bool {
        // SAFETY: Owned live event. A zero timeout never blocks video acquisition;
        // auto-reset consumes each native trigger notification once.
        unsafe { WaitForSingleObject(self.event, 0) == WAIT_OBJECT_0 }
    }

    pub(super) fn reset(&self) {
        // SAFETY: Owned live event; discard notifications from a stopped stream.
        let _ = unsafe { ResetEvent(self.event) };
    }
}

impl Drop for NativeButton {
    fn drop(&mut self) {
        let mut returned = 0;
        // SAFETY: Passing no request disables the registration identified by
        // its stable event-data buffer, before its handle or buffer is released.
        let _ = unsafe {
            self.control.KsEvent(
                ptr::null(),
                0,
                ptr::from_mut(self.data.as_mut()).cast(),
                self.data_len,
                &raw mut returned,
            )
        };
        // SAFETY: The worker owns this handle and closes it exactly once.
        let _ = unsafe { CloseHandle(self.event) };
    }
}
