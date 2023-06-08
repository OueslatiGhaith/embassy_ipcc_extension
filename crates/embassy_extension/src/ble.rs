use crate::{
    hci::{
        event::{FirmwareKind, Stm32Wb5xError, Stm32Wb5xEvent},
        RadioCoprocessor,
    },
    ipcc::Ipcc,
    tl_mbox::{shci::ShciBleInitCmdParam, TlMbox},
};
use bbqueue::BBBuffer;
use bluetooth_hci::{
    event::command::{CommandComplete, ReturnParameters},
    host::uart::{Error, Hci, Packet},
    Event,
};
use embassy_stm32::interrupt::{self, InterruptExt};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

type HeaplessEvtQueue = heapless::spsc::Queue<Packet<Stm32Wb5xEvent>, 32>;
pub type Rc = RadioCoprocessor<'static, BUFFER_SIZE>;

const BUFFER_SIZE: usize = 512;

static BB: BBBuffer<BUFFER_SIZE> = BBBuffer::new();
pub static mut RADIO_COPROCESSOR: *mut Rc = core::ptr::null_mut();

/// Type alias for the BLE stack's transport layer errors.
pub type BleTransportLayerError = bluetooth_hci::host::uart::Error<(), Stm32Wb5xError>;

/// BLE stack or system errors.
#[derive(Debug)]
pub enum BleError<E: core::fmt::Debug> {
    NbError(nb::Error<E>),
    EmptyError,
    UnexpectedEvent,
    NotInitialized,
}

impl<E: core::fmt::Debug> From<nb::Error<()>> for BleError<E> {
    fn from(_: nb::Error<()>) -> Self {
        BleError::EmptyError
    }
}

impl From<nb::Error<BleTransportLayerError>> for BleError<BleTransportLayerError> {
    fn from(e: nb::Error<bluetooth_hci::host::uart::Error<(), Stm32Wb5xError>>) -> Self {
        BleError::NbError(e)
    }
}

struct State {
    tx_int: Signal<CriticalSectionRawMutex, ()>,
    rx_int: Signal<CriticalSectionRawMutex, ()>,
}

static STATE: State = State {
    tx_int: Signal::new(),
    rx_int: Signal::new(),
};

pub struct Ble {
    rx_int: interrupt::IPCC_C1_RX,
    tx_int: interrupt::IPCC_C1_TX,
    deferred_events: HeaplessEvtQueue,
}

impl Ble {
    /// initializes the BLE stack and returns a status response from the BLE stack.
    pub async fn init(
        rx_int: interrupt::IPCC_C1_RX,
        tx_int: interrupt::IPCC_C1_TX,
        ble_config: ShciBleInitCmdParam,
        mbox: TlMbox,
        ipcc: Ipcc<'static>,
    ) -> Result<Self, BleError<Error<(), Stm32Wb5xError>>> {
        STATE.tx_int.reset();
        STATE.rx_int.reset();

        // register ISRs
        tx_int.disable();
        rx_int.disable();

        tx_int.set_handler(Self::on_tx_irq);
        tx_int.set_handler_context(core::ptr::null_mut());
        rx_int.set_handler(Self::on_rx_irq);
        rx_int.set_handler_context(core::ptr::null_mut());

        let (producer, consumer) = BB.try_split().unwrap();
        let mut rc = Rc::new(producer, consumer, mbox, ipcc, ble_config);
        unsafe {
            RADIO_COPROCESSOR = &mut rc;
        }

        tx_int.enable();
        rx_int.enable();

        let mut evt_queue = heapless::spsc::Queue::new();
        match Self::receive_event_helper(&mut evt_queue, &mut rc, false).await {
            Ok(Packet::Event(Event::Vendor(Stm32Wb5xEvent::CoprocessorReady(
                FirmwareKind::Wireless,
            )))) => Ok(Self {
                rx_int,
                tx_int,
                deferred_events: evt_queue,
            }),
            Err(e) => Err(BleError::NbError(e)),
            _ => Err(BleError::UnexpectedEvent),
        }
    }

    /// Sends an HCI BLE command and awaits for a response from the BLE stack.
    pub async fn perform_command(
        &mut self,
        command: impl Fn(&mut Rc) -> nb::Result<(), ()>,
    ) -> Result<ReturnParameters<Stm32Wb5xEvent>, BleError<Error<(), Stm32Wb5xError>>> {
        let rc = unsafe { RADIO_COPROCESSOR.as_mut() };
        if let Some(rc) = rc {
            cortex_m::interrupt::free(|_| command(rc))?;
            let response = Self::receive_event_helper(&mut self.deferred_events, rc, true).await?;
            if let Packet::Event(Event::CommandComplete(CommandComplete {
                return_params, ..
            })) = response
            {
                Ok(return_params)
            } else {
                Err(BleError::UnexpectedEvent)
            }
        } else {
            Err(BleError::NotInitialized)
        }
    }

    pub async fn receive_event(
        &mut self,
    ) -> Result<Packet<Stm32Wb5xEvent>, BleError<Error<(), Stm32Wb5xError>>> {
        let rc = unsafe { RADIO_COPROCESSOR.as_mut() };
        if let Some(rc) = rc {
            let event = Self::receive_event_helper(&mut self.deferred_events, rc, false).await;
            match event {
                Ok(event) => Ok(event),
                Err(_) => Err(BleError::UnexpectedEvent),
            }
        } else {
            Err(BleError::NotInitialized)
        }
    }

    /// returns `true` if there are some event(s) to be received
    pub fn has_events(&self) -> bool {
        STATE.rx_int.signaled() || self.deferred_events.peek().is_some()
    }

    async fn receive_event_helper(
        queue: &mut HeaplessEvtQueue,
        rc: &mut Rc,
        need_cmd_response: bool,
    ) -> nb::Result<Packet<Stm32Wb5xEvent>, Error<(), Stm32Wb5xError>> {
        loop {
            let event = cortex_m::interrupt::free(|_| {
                rc.process_events();
                rc.read().ok()
            });

            // If the receiver is only interested in command response events,
            // it will be an error to return the first event from the event queue since
            // it is not guaranteed that no events occurred in between of command execution and
            // response.
            // Thus we defer all of the non-command-response events into the temporary queue before
            // we get a command response event that will be returned.
            if need_cmd_response {
                if let Some(event) = event {
                    if let Packet::Event(Event::CommandComplete(_)) = event {
                        return Ok(event);
                    } else {
                        // Defer the currently received event into temporary queue
                        // for it to be processed later
                        queue.enqueue(event).unwrap();
                    }
                }
            } else {
                let event = queue.dequeue().or(event);
                if let Some(event) = event {
                    return Ok(event);
                }
            }

            STATE.rx_int.wait().await;
        }
    }

    unsafe fn on_tx_irq(_ctx: *mut ()) {
        if let Some(rc) = RADIO_COPROCESSOR.as_mut() {
            rc.handle_ipcc_tx();
        }

        STATE.tx_int.signal(());
    }

    unsafe fn on_rx_irq(_ctx: *mut ()) {
        if let Some(rc) = RADIO_COPROCESSOR.as_mut() {
            rc.handle_ipcc_rx();
        }

        STATE.rx_int.signal(());
    }
}

impl Drop for Ble {
    fn drop(&mut self) {
        self.rx_int.disable();
        self.rx_int.remove_handler();

        self.tx_int.disable();
        self.tx_int.remove_handler();

        STATE.rx_int.reset();
        STATE.tx_int.reset();
    }
}
