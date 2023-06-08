use core::time::Duration;

use bluetooth_hci::{
    event::command::{CommandComplete, ReturnParameters},
    host::{
        uart::{Hci, Packet},
        AdvertisingFilterPolicy, Hci as HostHci, OwnAddressType,
    },
    types::AdvertisingType,
    Event,
};
use rf::hci::{
    command::{
        gap::{DiscoverableParameters, GapCommands, IoCapability, LocalName, Role},
        gatt::{GattCommands, UpdateCharacteristicValueParameters},
        hal::{ConfigData, HalCommands},
    },
    event::{self, Stm32Wb5xError, Stm32Wb5xEvent},
};

use crate::{
    utils::{get_bd_addr, get_erk, get_irk, get_random_addr},
    Rc, STATE,
};

#[derive(defmt::Format)]
struct OtherErrWrapper(#[defmt(Debug2Format)] bluetooth_hci::host::uart::Error<(), Stm32Wb5xError>);

fn log_response_err(error: nb::Error<bluetooth_hci::host::uart::Error<(), Stm32Wb5xError>>) {
    match error {
        nb::Error::WouldBlock => {
            // skip would block, as it will be resolved when the RX interruption is called
        }
        nb::Error::Other(err) => {
            let wrapper = OtherErrWrapper(err);
            defmt::warn!("Err {}", wrapper);
        }
    }
}

pub async fn receive_event_helper(
    rc: &mut Rc,
    need_cmd_rsp: bool,
) -> Result<Packet<Stm32Wb5xEvent>, ()> {
    loop {
        let event = cortex_m::interrupt::free(|_| {
            rc.process_events();
            match rc.read() {
                Ok(data) => Some(data),
                Err(err) => {
                    log_response_err(err);
                    None
                }
            }
        });

        // If the receiver is only interested in command response events,
        // it will be an error to return the first event from the event queue since
        // it is not guaranteed that no events occurred in between of command execution and
        // response.
        // Thus we defer all of the non-command-response events into the temporary queue before
        // we get a command response event that will be returned.

        if need_cmd_rsp {
            if let Some(event) = event {
                if let Packet::Event(Event::CommandComplete(_)) = event {
                    log_event(&event);

                    return Ok(event);
                } else {
                    log_event(&event);
                    // Defer the currently received event into temporary queue
                    // for it to be processed later
                }
            }
        } else if let Some(event) = event {
            log_event(&event);
            return Ok(event);
        }

        STATE.rx_int.wait().await;
    }
}

#[derive(defmt::Format)]
struct EventWrapper(#[defmt(Debug2Format)] Packet<Stm32Wb5xEvent>);

fn log_event(event: &Packet<Stm32Wb5xEvent>) {
    let wrapper = EventWrapper(event.clone());
    defmt::warn!("{}", wrapper)
}

pub async fn init_hal(rc: &mut Rc, device_name: &[u8]) -> Result<(), nb::Error<()>> {
    defmt::info!("reset");
    rc.reset()?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("public address");
    rc.write_config_data(&ConfigData::public_address(get_bd_addr()).build())?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("random address");
    rc.write_config_data(&ConfigData::random_address(get_random_addr()).build())?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("identity root key");
    rc.write_config_data(&ConfigData::identity_root(&get_irk()).build())?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("encryption root key");
    rc.write_config_data(&ConfigData::encryption_root(&get_erk()).build())?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("GATT init");
    rc.init_gatt()?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("GAP init");
    rc.init_gap(Role::PERIPHERAL, false, device_name.len() as u8)?;
    let response = receive_event_helper(rc, true).await?;

    let gap = if let Packet::Event(Event::CommandComplete(CommandComplete {
        return_params: ReturnParameters::Vendor(event::command::ReturnParameters::GapInit(gap)),
        ..
    })) = response
    {
        gap
    } else {
        return Err(nb::Error::Other(()));
    };

    defmt::info!("update device name");
    rc.update_characteristic_value(&UpdateCharacteristicValueParameters {
        service_handle: gap.service_handle,
        characteristic_handle: gap.dev_name_handle,
        offset: 0,
        value: device_name,
    })
    .map_err(|_| ())?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("set io capability");
    rc.set_io_capability(IoCapability::DisplayConfirm)?;
    let _ = receive_event_helper(rc, true).await?;

    defmt::info!("set scan response data");
    rc.le_set_scan_response_data(&[]).map_err(|_| ())?;
    let _ = receive_event_helper(rc, true).await?;

    Ok(())
}

pub async fn set_discoverable(rc: &mut Rc, local_name: &[u8]) -> Result<(), nb::Error<()>> {
    defmt::info!("set discoverable");
    let discovery_params = DiscoverableParameters {
        advertising_type: AdvertisingType::ConnectableUndirected,
        advertising_interval: Some((Duration::from_millis(100), Duration::from_millis(100))),
        address_type: OwnAddressType::Public,
        filter_policy: AdvertisingFilterPolicy::AllowConnectionAndScan,
        local_name: Some(LocalName::Complete(local_name)),
        advertising_data: &[],
        conn_interval: (None, None),
    };

    rc.set_discoverable(&discovery_params).map_err(|_| ())?;
    let _ = receive_event_helper(rc, true).await?;

    Ok(())
}
