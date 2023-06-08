//! Bluetooth HCI for STMicro's STM32WB5x Bluetooth controllers.
//!
//! # Design
//!
//! The STM32WB55 is a dual-core SoC that contains application controller (Cortex-M4F) and
//! radio coprocessor (Cortex-M0+). This crate is intended to run on application controller and
//! communicates with the BLE stack that is running on radio coprocessor. The communication is
//! performed through mailbox interface based on shared SRAM area and IPCC peripheral interrupts.
//!
//! This crate defines a public struct, [`RadioCoprocessor`] that owns the IPCC peripheral,
//! implements IPCC IRQ handlers and and implements [`bluetooth_hci::Controller`],
//! which provides access to the full Bluetooth HCI.
//!
//! STM32WB55 BLE stack implements 4.x and 5.x versions of the Bluetooth [specification].
//!
//!
//! # Vendor-Specific Commands
//!
//! STM32WB5x provides several vendor-specific commands that control the behavior of the
//! controller.
//!
//! # Vendor-Specific Events
//!
//! STM32WB5x provides several vendor-specific events that provide data related to the
//! controller. Many of these events are forwarded from the link layer, and these are documented
//! with a reference to the appropriate section of the Bluetooth specification.
//!
//! [specification]: https://www.bluetooth.com/specifications/bluetooth-core-specification

use bbqueue::{Consumer, Producer};
use bluetooth_hci::{
    host::{uart::CommandHeader, HciHeader},
    Controller, Opcode,
};

pub use bluetooth_hci::host::{AdvertisingFilterPolicy, AdvertisingType, OwnAddressType};

use crate::{
    ipcc::Ipcc,
    tl_mbox::{self, cmd::CmdSerial, consts::TlPacketType, shci::ShciBleInitCmdParam, TlMbox},
};

pub mod command;
pub mod event;
pub mod opcode;

const TX_BUF_SIZE: usize = core::mem::size_of::<CmdSerial>();

/// handle for interfacing with the STM32WB5x radio coprocessor
pub struct RadioCoprocessor<'buf, const N: usize> {
    mbox: TlMbox,
    ipcc: Ipcc<'buf>,
    config: ShciBleInitCmdParam,
    buff_producer: Producer<'buf, N>,
    buff_consumer: Consumer<'buf, N>,
    tx_buf: [u8; TX_BUF_SIZE],
    is_ble_ready: bool,
}

impl<'buf, const N: usize> RadioCoprocessor<'buf, N> {
    /// creates a new [`RadioCoprocessor`] instance to send commands and to
    /// receive events from
    pub fn new(
        producer: Producer<'buf, N>,
        consumer: Consumer<'buf, N>,
        mbox: TlMbox,
        ipcc: Ipcc<'buf>,
        config: ShciBleInitCmdParam,
    ) -> Self {
        Self {
            mbox,
            ipcc,
            config,
            buff_producer: producer,
            buff_consumer: consumer,
            tx_buf: [0u8; TX_BUF_SIZE],
            is_ble_ready: false,
        }
    }

    fn write_command(&mut self, opcode: Opcode, params: &[u8]) -> nb::Result<(), ()> {
        const HEADER_LEN: usize = 4;
        let mut header = [0; HEADER_LEN];
        CommandHeader::new(opcode, params.len()).copy_into_slice(&mut header);

        self.write(&header, params)
    }

    /// call this function from `IPCC_C1_RX` interrupt context
    pub fn handle_ipcc_rx(&mut self) {
        self.mbox.interrupt_ipcc_rx_handler(&mut self.ipcc);
    }

    /// call this function from `IPCC_C1_TX` interrupt context
    pub fn handle_ipcc_tx(&mut self) {
        self.mbox.interrupt_ipcc_tx_handler(&mut self.ipcc);
    }

    /// call this function outside of interrupt context, for example in `main()` loop.
    /// Returns `true` if events were written and can be read with HCI `read()` function.
    /// returns `false` if no HCI events were written
    pub fn process_events(&mut self) -> bool {
        while let Some(evt) = self.mbox.dequeue_event() {
            defmt::debug!("processing event");

            let event = evt.evt();

            let mut buf = self
                .buff_producer
                .grant_exact(evt.size().expect("Known packet kind"))
                .expect("No space in buffer");

            evt.write(buf.buf()).expect("EVT_BUF_SIZE is too small");

            if event.kind() == 18 {
                defmt::debug!("processing event `coprocessor ready` detected");
                tl_mbox::shci::shci_ble_init(&mut self.ipcc, self.config);
                self.is_ble_ready = true;
                buf.buf()[0] = 0x04; // replace event code with one that is supported by HCI
            }

            buf.commit(evt.size().unwrap());
        }

        if self.mbox.pop_last_cc_evt().is_some() {
            defmt::debug!("processing events cc event detected");
            return false;
        }

        true
    }
}

impl<'buf, const N: usize> bluetooth_hci::Controller for RadioCoprocessor<'buf, N> {
    type Error = ();
    type Header = bluetooth_hci::host::uart::CommandHeader;
    type Vendor = STM32WB5xTypes;

    fn write(&mut self, header: &[u8], payload: &[u8]) -> nb::Result<(), Self::Error> {
        let cmd_code = header[0];
        let cmd = TlPacketType::try_from(cmd_code).map_err(|_| ())?;

        self.tx_buf = [0; TX_BUF_SIZE];
        self.tx_buf[..header.len()].copy_from_slice(header);
        self.tx_buf[header.len()..(header.len() + payload.len())].copy_from_slice(payload);

        match &cmd {
            TlPacketType::AclData => {
                // Destination buffer: ble table, phci_acl_data_buffer, acldataserial field
                todo!()
            }

            TlPacketType::SysCmd => {
                // Destination buffer: SYS table, pcmdbuffer, cmdserial field
                todo!()
            }

            _ => {
                tl_mbox::ble::ble_send_cmd(&mut self.ipcc, &self.tx_buf[..]);
            }
        }

        Ok(())
    }

    fn read_into(&mut self, buffer: &mut [u8]) -> nb::Result<(), Self::Error> {
        match self.buff_consumer.read() {
            Ok(grant) => {
                if buffer.len() <= grant.buf().len() {
                    buffer.copy_from_slice(&grant.buf()[..buffer.len()]);
                    grant.release(buffer.len());

                    Ok(())
                } else {
                    Err(nb::Error::WouldBlock)
                }
            }
            Err(bbqueue::Error::InsufficientSize) => Err(nb::Error::WouldBlock),
            Err(_) => Err(nb::Error::Other(())),
        }
    }

    fn peek(&mut self, n: usize) -> nb::Result<u8, Self::Error> {
        match self.buff_consumer.read() {
            Ok(grant) => {
                if n >= grant.buf().len() {
                    Err(nb::Error::WouldBlock)
                } else {
                    Ok(grant.buf()[n])
                }
            }
            Err(bbqueue::Error::InsufficientSize) => Err(nb::Error::WouldBlock),
            Err(_) => Err(nb::Error::Other(())),
        }
    }
}

/// specify vendor specifi extensions for BlueNRG
pub struct STM32WB5xTypes;
impl bluetooth_hci::Vendor for STM32WB5xTypes {
    type Status = event::Status;
    type Event = event::Stm32Wb5xEvent;
}

/// master trait that encompasses all commands, and communicats over UART
pub trait UartController<E>:
    crate::hci::command::gap::GapCommands<Error = E>
    + crate::hci::command::gatt::GattCommands<Error = E>
    + crate::hci::command::hal::HalCommands<Error = E>
    + crate::hci::command::l2cap::L2capCommands<Error = E>
    + bluetooth_hci::host::uart::Hci<
        E,
        crate::hci::event::Stm32Wb5xEvent,
        crate::hci::event::Stm32Wb5xError,
    >
{
}

impl<T, E> UartController<E> for T where
    T: crate::hci::command::gap::GapCommands<Error = E>
        + crate::hci::command::gatt::GattCommands<Error = E>
        + crate::hci::command::hal::HalCommands<Error = E>
        + crate::hci::command::l2cap::L2capCommands<Error = E>
        + bluetooth_hci::host::uart::Hci<
            E,
            crate::hci::event::Stm32Wb5xEvent,
            crate::hci::event::Stm32Wb5xError,
        >
{
}

/// vendor specific interpretation of the local version information from the controlleer
#[derive(Clone)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

/// extension trait to convert [`bluetooth_hci::event::command::LocalVersionInfo`] into
/// the BLE stack specific [`Version`] struct
pub trait LocalVersionInfoExt {
    /// converts [`LocalVersionInfo`] as return by the controller into a BLE stack
    /// specific [`Version`] struct
    fn wireless_fw_info(&self) -> Version;
}

impl<VS> LocalVersionInfoExt for bluetooth_hci::event::command::LocalVersionInfo<VS> {
    fn wireless_fw_info(&self) -> Version {
        // TODO
        Version {
            major: self.hci_version,
            minor: ((self.lmp_subversion >> 4) & 0xF) as u8,
            patch: (self.lmp_subversion & 0xF) as u8,
        }
    }
}
