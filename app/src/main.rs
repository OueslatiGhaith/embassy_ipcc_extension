#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use bbqueue::BBBuffer;
use bluetooth_hci::{host::uart::Packet, Event};
use embassy_executor::Spawner;
use embassy_stm32::interrupt::{self, InterruptExt};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embedded_alloc::Heap;
use rf::{
    ble::RADIO_COPROCESSOR,
    hci::{
        command::gatt::{GattCommands, WriteResponseParameters},
        event::{FirmwareKind, Stm32Wb5xEvent},
        RadioCoprocessor,
    },
    ipcc::Ipcc,
    tl_mbox::{shci::ShciBleInitCmdParam, TlMbox},
};

use crate::{
    gatt::init_gatt_services,
    helpers::{init_hal, receive_event_helper, set_discoverable},
};

mod gatt;
mod helpers;
mod utils;

use {defmt_rtt as _, panic_probe as _};

#[global_allocator]
static HEAP: Heap = Heap::empty();

pub type Rc = RadioCoprocessor<'static, BUFFER_SIZE>;

const BUFFER_SIZE: usize = 512;
static BB: BBBuffer<BUFFER_SIZE> = BBBuffer::new();

pub struct State {
    tx_int: Signal<CriticalSectionRawMutex, ()>,
    rx_int: Signal<CriticalSectionRawMutex, ()>,
}

pub static STATE: State = State {
    tx_int: Signal::new(),
    rx_int: Signal::new(),
};

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    // Initialize the allocator BEFORE you use it
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 4096;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }

    let p = embassy_stm32::init(Default::default());

    let config = rf::ipcc::Config::default();
    let rx_irq = interrupt::take!(IPCC_C1_RX);
    let tx_irq = interrupt::take!(IPCC_C1_TX);

    let mut ipcc = Ipcc::new(p.IPCC, config);
    let mbox = TlMbox::init(&mut ipcc);

    let config = ShciBleInitCmdParam {
        p_ble_buffer_address: 0,
        ble_buffer_size: 0,
        num_attr_record: 68,
        num_attr_serv: 8,
        attr_value_arr_size: 1344,
        num_of_links: 2,
        extended_packet_length_enable: 1,
        pr_write_list_size: 0x3A,
        mb_lock_count: 0x79,
        att_mtu: 156,
        slave_sca: 500,
        master_sca: 0,
        ls_source: 1,
        max_conn_event_length: 0xFFFFFFFF,
        hs_startup_time: 0x148,
        viterbi_enable: 1,
        ll_only: 0,
        hw_version: 0,
    };

    STATE.tx_int.reset();
    STATE.rx_int.reset();

    tx_irq.disable();
    rx_irq.disable();

    tx_irq.set_handler(|_| unsafe {
        if let Some(rc) = RADIO_COPROCESSOR.as_mut() {
            rc.handle_ipcc_tx();
        }
        STATE.tx_int.signal(());
    });
    rx_irq.set_handler(|_| unsafe {
        if let Some(rc) = RADIO_COPROCESSOR.as_mut() {
            rc.handle_ipcc_rx();
        }
        STATE.rx_int.signal(());
    });

    let (producer, consumer) = BB.try_split().unwrap();
    let mut rc = Rc::new(producer, consumer, mbox, ipcc, config);
    unsafe {
        RADIO_COPROCESSOR = &mut rc;
    }

    tx_irq.enable();
    rx_irq.enable();

    let res = receive_event_helper(&mut rc, false).await;
    if let Ok(Packet::Event(Event::Vendor(Stm32Wb5xEvent::CoprocessorReady(
        FirmwareKind::Wireless,
    )))) = res
    {
        defmt::info!("starting BLE");
    }

    init_hal(&mut rc, b"STM32WB55RGVx").await.unwrap();
    let _ble_context = init_gatt_services(&mut rc).await.unwrap();
    set_discoverable(&mut rc, b"STM32WB55RGVx").await.unwrap();

    defmt::info!("done");

    loop {
        let return_params = receive_event_helper(&mut rc, false).await;

        if let Ok(Packet::Event(event)) = return_params {
            match event {
                Event::Vendor(vendor_event) => match vendor_event {
                    Stm32Wb5xEvent::AttReadPermitRequest(read_req) => {
                        defmt::info!("allowing read");
                        rc.allow_read(read_req.conn_handle).unwrap();
                        let _ = receive_event_helper(&mut rc, true).await;
                    }
                    Stm32Wb5xEvent::AttWritePermitRequest(write_req) => {
                        defmt::info!("allowing write");
                        rc.write_response(&WriteResponseParameters {
                            attribute_handle: write_req.attribute_handle,
                            conn_handle: write_req.conn_handle,
                            status: Ok(()),
                            value: write_req.value(),
                        })
                        .unwrap();
                        let _ = receive_event_helper(&mut rc, false).await;
                    }
                    // Stm32Wb5xEvent::GattAttributeModified(_attribute) => {
                    //     Timer::after(embassy_time::Duration::from_millis(2000)).await;

                    //     defmt::info!("sending a notification");
                    //     rc.update_characteristic_value(&UpdateCharacteristicValueParameters {
                    //         characteristic_handle: ble_context.notify_char_handle,
                    //         service_handle: ble_context.service_handle,
                    //         offset: 0,
                    //         value: b"hello world",
                    //     })
                    //     .unwrap();
                    //     let _ = receive_event_helper(&mut rc, false).await;
                    // }
                    _ => {}
                },
                Event::DisconnectionComplete(_) => {
                    defmt::info!("disconnected, readvertising");
                    set_discoverable(&mut rc, b"STM32WB55RGVx").await.unwrap();
                }
                _ => {}
            }
        }
    }
}
